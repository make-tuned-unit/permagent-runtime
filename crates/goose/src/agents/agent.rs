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
    check_if_compaction_needed, compact_messages_accounted, maybe_summarize_tool_pairs_accounted,
    DEFAULT_COMPACTION_THRESHOLD,
};
use crate::conversation::message::{
    ActionRequiredData, Message, MessageContent, ProviderMetadata, SystemNotificationType,
    ToolRequest,
};
use crate::conversation::{
    debug_conversation_fix, fix_conversation, merge_consecutive_messages, Conversation,
};
use crate::cost_router::cache::SystemPromptParts;
use crate::mcp_utils::ToolResult;
use crate::permission::permission_inspector::PermissionInspector;
use crate::permission::permission_judge::PermissionCheckResult;
use crate::permission::PermissionConfirmation;
use crate::providers::base::{PermissionRouting, Provider, ProviderUsage};
use crate::providers::errors::ProviderError;
use crate::recipe::{Author, Recipe, Response, Settings};
use crate::scheduler_trait::SchedulerTrait;
use crate::security::adversary_inspector::AdversaryInspector;
use crate::security::egress_inspector::EgressInspector;
use crate::security::security_inspector::SecurityInspector;
use crate::security::write_jail::WriteJailInspector;
use crate::session::extension_data::{EnabledExtensionsState, ExtensionState};
use crate::session::turn_timing::TurnTimingRecorder;
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

/// How many times one reply re-prompts after an assistant turn that produced
/// no text and no tool call before it lets the run end.
const MAX_EMPTY_TURN_CONTINUATIONS: u32 = 2;

/// The continuation sent after an empty assistant turn.
const EMPTY_TURN_CONTINUATION: &str = "Continue: take the next concrete step with a tool call.";

/// The continuation sent when the empty turn was cut off by the output cap.
const EMPTY_TURN_OUTPUT_CAP_CONTINUATION: &str =
    "Your last response ran out of output room while reasoning, before it wrote any text \
     or tool call. Continue: take the next concrete step with a tool call, and keep your \
     reasoning short.";

/// What the reply loop does with a turn that called no tool.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EmptyTurnDecision {
    /// The turn said something or called a tool: the ordinary end-of-turn
    /// handling applies.
    NotEmpty,
    /// Nothing came back: re-prompt the same session with this message.
    Continue { prompt: &'static str },
    /// Still nothing after the allowed continuations: end the run, and say why.
    GiveUp { reason: String },
}

/// Decide what to do with an assistant turn, given what it produced.
///
/// A turn with no text and no tool request is not the model finishing. On
/// 2026-09-25 two gpt-5.4-mini goal workers each returned a turn holding only
/// a reasoning item (the output cap was spent thinking, the response was
/// `incomplete`), and the loop took that as the end of the run: the worker
/// exited mid-task, the done-check held it, and the goal went back to Ready
/// to start over. Such a turn gets a short continuation instead, at most
/// [`MAX_EMPTY_TURN_CONTINUATIONS`] times in a row, and then the run ends
/// with a stated reason rather than silently.
///
/// `incomplete_reason` is the provider's own reason for stopping early
/// (`ProviderUsage::incomplete_reason`), when it gave one.
fn decide_empty_turn(
    produced_text: bool,
    requested_tool: bool,
    incomplete_reason: Option<&str>,
    continuations_so_far: u32,
) -> EmptyTurnDecision {
    if produced_text || requested_tool {
        return EmptyTurnDecision::NotEmpty;
    }
    if continuations_so_far >= MAX_EMPTY_TURN_CONTINUATIONS {
        return EmptyTurnDecision::GiveUp {
            reason: format!(
                "{} assistant turns in a row produced no text and no tool call{}; ending the run",
                continuations_so_far + 1,
                incomplete_reason
                    .map(|r| format!(" (last response incomplete: {r})"))
                    .unwrap_or_default()
            ),
        };
    }
    let prompt = if incomplete_reason
        == Some(crate::providers::formats::openai_responses::INCOMPLETE_MAX_OUTPUT_TOKENS)
    {
        EMPTY_TURN_OUTPUT_CAP_CONTINUATION
    } else {
        EMPTY_TURN_CONTINUATION
    };
    EmptyTurnDecision::Continue { prompt }
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
    /// R48c (harness loop, "Lane R48c"): true for a dispatched goal worker
    /// running the compact prompt profile. `new()` defaults this to `false`
    /// so every existing call site is unaffected — only `with_compact_
    /// worker_profile` (goal_engine.rs's `InternalSubagentEngine::spawn`)
    /// sets it. Read by `Agent::list_tools` to drop the always-on
    /// `platform__load_feature_lesson` tool, which is appended outside the
    /// extension-grant mechanism entirely and so is otherwise unreachable
    /// by the profile's tool-set narrowing.
    pub compact_worker_profile: bool,
    /// Only the headless coding recipe's top-level coordinator. Goal workers
    /// get fresh AgentRunnerConfig values and retain their own tool grants.
    pub headless_coding_coordinator: bool,
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
            compact_worker_profile: false,
            headless_coding_coordinator: false,
        }
    }

    /// R48c: opt a subagent run into the compact worker prompt profile's
    /// tool-set effect at the `Agent::list_tools` layer. See the field's own
    /// doc comment for why this can't just be another `new()` parameter's
    /// worth of plumbing — it's set at exactly one call site.
    pub fn with_compact_worker_profile(mut self, compact: bool) -> Self {
        self.compact_worker_profile = compact;
        self
    }
}

/// Whether THIS reply should open a new budget task.
///
/// Interactive only, deliberately: a child or headless session inherits its
/// parent's identity at creation, so opening another here would split one
/// user task into several budget identities and defeat the per-task ceiling.
///
/// The exception that cost us: a SCHEDULED session has no parent to inherit
/// from — the scheduler mints it from nothing — so it fell through this rule
/// with no task at all and every paid call was refused ("paid provider
/// reservation requires a durable task identity"). That is fixed where the run
/// is created (`scheduler.rs`, at `create_session`), not by loosening this
/// rule: the run boundary is the scheduler's to declare, and one run is one
/// task. See `a_scheduled_run_opens_its_own_budget_task`.
fn starts_new_budget_task(session_type: crate::session::SessionType, message_text: &str) -> bool {
    session_type.is_interactive()
        && !crate::agents::execute_commands::COMPACT_TRIGGERS.contains(&message_text.trim())
}

/// The main goose Agent
/// A time-boxed fallback in force after a transient provider failure. The
/// session record was never changed (the switch is in memory only), so an
/// evicted or restarted agent comes back on the operator's model regardless.
struct TransientFailover {
    /// The provider the session was on, and goes back to.
    original: Arc<dyn Provider>,
    /// The stand-in that was installed, so the switch back only happens while
    /// it is still the live provider.
    fallback: Arc<dyn Provider>,
    fallback_target: crate::cost_router::role_map::RoleModel,
    until: std::time::Instant,
}

/// Wait out a same-model retry, visibly (the shared provider wait status the
/// CLI and desktop render) and cancellably.
async fn sleep_before_same_model_retry(
    provider: &str,
    err: &ProviderError,
    delay: std::time::Duration,
    attempt: usize,
    cancel_token: &Option<CancellationToken>,
) {
    use crate::providers::wait_status;
    wait_status::publish(wait_status::ProviderWait {
        provider: provider.to_string(),
        reason: match err {
            ProviderError::RateLimitExceeded { .. } => wait_status::WaitReason::RateLimited,
            ProviderError::ServerError(_) => wait_status::WaitReason::ServerError,
            _ => wait_status::WaitReason::Transient,
        },
        started: std::time::Instant::now(),
        delay,
        attempt,
        max_attempts: crate::cost_router::fallback::SAME_MODEL_MAX_RETRIES,
    });
    match cancel_token {
        Some(token) => {
            tokio::select! {
                _ = token.cancelled() => {}
                _ = tokio::time::sleep(delay) => {}
            }
        }
        None => tokio::time::sleep(delay).await,
    }
    wait_status::clear();
}

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
    /// Hooks consulted at the one point the reply loop decides a turn is over.
    /// The `ToolInspector` chain covers "before a tool runs"; this covers
    /// "the model thinks it is finished" — see `crate::after_turn`.
    pub(super) after_turn_manager: crate::after_turn::AfterTurnManager,
    container: Mutex<Option<Container>>,
    /// mtime of the config file at the last extension sync. Lets the
    /// resident-agent path answer "anything new?" with a `stat` instead of a
    /// full config read + YAML parse on every request. See
    /// `sync_extensions_with_config`.
    last_extension_config_mtime: Mutex<Option<std::time::SystemTime>>,
    /// R46a (harness loop, "A43 analysis" lever 3): the raw provider-error
    /// text behind this agent's most recent context-length-exceeded give-up
    /// (recorded only at that one arm of the reply loop — see
    /// `record_context_length_failure`). A caller that drives an `Agent`
    /// headless (`delegate`/`delegate_many`'s spawned subagent, via
    /// `subagent_handler::get_agent_messages`) reads this AFTER the reply
    /// stream ends to turn a bare `AgentRuntimeOutcome::Failed` into a
    /// classified reason instead of leaving the coordinator with an opaque
    /// `-32603`. Never read by the ordinary user-facing reply path — the
    /// turn's own visible text stays scrubbed of raw provider payloads,
    /// unchanged.
    last_context_length_failure: Mutex<Option<String>>,
    /// A fallback taken for a TRANSIENT provider failure, and when it ends.
    /// Set only by `fail_over_before_commit`; taken back at the next loop
    /// iteration after `until`, and always at the start of the next reply, so
    /// a blip moves the session for a bounded time instead of for the rest of
    /// the session (20260924_3). Cleared by `update_provider`: any explicit
    /// switch, including a permanent-failure fallback, supersedes it.
    transient_failover: Mutex<Option<TransientFailover>>,
    /// Test seam for the mid-turn switch: when set, the next
    /// `switch_to_permanent_failure_fallback` uses this target and provider
    /// instead of probing config and building one by name, so the reply-loop
    /// tests can drive a real failover with in-memory providers.
    #[cfg(test)]
    test_failover: Mutex<Option<(crate::cost_router::role_map::RoleModel, Arc<dyn Provider>)>>,
    /// Every `request_tokens` the mid-turn switch was called with, recorded
    /// before the seam above replaces the choice, so a reply-loop test can
    /// prove the arms pass the failed request's real size to it.
    #[cfg(test)]
    test_failover_request_tokens: Mutex<Vec<Option<usize>>>,
}

/// The headless coding root plans and supervises; dispatched workers receive
/// fresh agent configs and keep their own implementation/verification tools.
/// Match full public names so a same-named tool from another extension cannot
/// inherit a coordinator grant.
pub(super) fn headless_coding_coordinator_allows_tool(name: &str) -> bool {
    matches!(
        name,
        "read"
            | "tree"
            | "search"
            | "map_query"
            | "analyze"
            | "delegate"
            | "orchestrator__routing_roster"
            | "orchestrator__check_worker"
            | "orchestrator__decompose_roadmap"
            | "orchestrator__create_roadmap"
            | "orchestrator__goal_advance"
            | "orchestrator__goal_status"
            | "orchestrator__goal_list"
            | "orchestrator__resume_roadmap"
            | "orchestrator__escalate"
    ) || name == FINAL_OUTPUT_TOOL_NAME
}

#[derive(Clone, Debug)]
pub enum AgentEvent {
    Message(Message),
    McpNotification((String, ServerNotification)),
    HistoryReplaced(Conversation),
    /// Structured terminal outcome for the reply stream. Consumers that
    /// project run telemetry must use this per-stream event instead of the
    /// process-global HUD runtime registry.
    RuntimeOutcome(AgentRuntimeOutcome),
}

/// Outcome of one concrete [`Agent::reply`] stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentRuntimeOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

/// #348 — real agent lifecycle hook for the World View. This Drop guard ties the
/// primary agent's runtime state to the ACTUAL reply turn: `working` on entry,
/// and on drop `error` (a real provider failure that set the latch) or else
/// `done` → available. Covers every exit path — clean break, max-turns,
/// cancellation, panic. The status poll and the agent-state tick read this
/// registry, so a real error survives instead of being clobbered by the 2s
/// session-activity derive.
///
/// TWO FACTS, NOT ONE. `errored` is deliberately STICKY for the whole turn —
/// that stickiness is what the HUD needs, per the paragraph above. But the
/// turn's [`AgentRuntimeOutcome`] answers a different question: how did the turn
/// END. Reading the sticky latch for that was wrong. `mark_error` breaks only
/// the INNER stream loop; when tools were already called the outer loop
/// iterates again and can succeed, so a turn that recovered still reported
/// `Failed` — to `subagent_handler` (a recovered worker read by its coordinator
/// as failed), to the HUD, and, once the scheduler starts reading the outcome,
/// to the three-strike auto-pause, which would pause a working job.
///
/// So `iteration_errored` is cleared by [`Self::begin_iteration`] when the
/// entry guards allow another outer-loop attempt and is what the outcome reads. Keeping both in
/// the guard means the 15 `mark_error` call sites need no edits and a new
/// failure path gets the correct behaviour for free.
///
/// Hoisted out of `reply_internal` so the contract is testable; it was a
/// function-local type with no test coverage at all.
pub(crate) struct ReplyStateGuard {
    errored: bool,
    iteration_errored: bool,
}

impl ReplyStateGuard {
    pub(crate) fn start() -> Self {
        crate::events::record_agent_working("henry");
        Self {
            errored: false,
            iteration_errored: false,
        }
    }

    /// Called at the top of each outer-loop turn iteration. Clears ONLY the
    /// per-iteration fact; the HUD's sticky latch survives on purpose.
    pub(crate) fn begin_iteration(&mut self) {
        self.iteration_errored = false;
    }

    pub(crate) fn mark_error(&mut self) {
        self.errored = true;
        self.iteration_errored = true;
    }

    /// Did the LAST iteration fail? This — not the sticky latch — is the turn's
    /// outcome.
    pub(crate) fn iteration_errored(&self) -> bool {
        self.iteration_errored
    }

    #[cfg(test)]
    pub(crate) fn errored_for_hud(&self) -> bool {
        self.errored
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

/// The turn's verdict. Cancellation outranks a clean iteration because the
/// scheduler's kill path records the outcome of a cancelled run itself — two
/// writers for one fact is worse than one coarse one. Pure so the truth table
/// is testable without a provider.
pub(crate) fn runtime_outcome(iteration_errored: bool, cancelled: bool) -> AgentRuntimeOutcome {
    if iteration_errored {
        AgentRuntimeOutcome::Failed
    } else if cancelled {
        AgentRuntimeOutcome::Cancelled
    } else {
        AgentRuntimeOutcome::Succeeded
    }
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
/// Fold ONE turn's newly produced messages into an alternating sequence.
///
/// The provider stream can hand back several assistant messages for a single
/// turn (reasoning item, then tool-call item). Persisting them separately puts
/// consecutive same-role messages into the conversation, which every provider
/// rejects and which the repair layer then has to merge on every later turn.
///
/// Returns the folded messages and how many merges it took (0 on a turn that
/// was already well-formed — the common case for providers that stream a whole
/// assistant turn as one message).
pub(crate) fn coalesce_turn_messages(messages: Vec<Message>) -> (Vec<Message>, usize) {
    let (merged, issues) = merge_consecutive_messages(messages);
    // Consecutive-role merge concatenates content arrays, which can leave
    // adjacent Text/Thinking parts in one message — the same split-delta
    // shape that made MOIM's issue list grow every turn of session
    // 20260827_1. Fold those here so the persisted turn is already clean.
    let folded: Vec<Message> = merged
        .into_iter()
        .map(Message::coalesce_adjacent_text_and_thinking)
        .collect();
    (folded, issues.len())
}

/// The assistant side of a tool-calling response, as history keeps it.
#[derive(Debug)]
pub(crate) struct ToolCallHistory {
    /// Pushed ahead of the calls, and only when no call parsed: the response's
    /// reasoning and text then have no tool-call message to ride on.
    pub(crate) lead: Option<Message>,
    /// One entry per request, in order: the message that carries the call, or
    /// `None` for a call that did not parse (the reply loop answers those
    /// with a recovery observation instead).
    pub(crate) calls: Vec<Option<Message>>,
}

/// A message this provider call streamed ahead of the tool-calling response:
/// an assistant message with no tool traffic. The reply loop pushes those as
/// they arrive, and every tool branch ends on a tool result or a recovery
/// observation (both user messages), so the trailing run of them in the
/// turn's unsaved messages is what preceded the current response.
fn is_streamed_ahead_of_tool_calls(message: &Message) -> bool {
    message.role == rmcp::model::Role::Assistant
        && !message.content.iter().any(|content| {
            matches!(
                content,
                MessageContent::ToolRequest(_)
                    | MessageContent::FrontendToolRequest(_)
                    | MessageContent::ToolResponse(_)
            )
        })
}

fn joined_thinking(content: &[MessageContent]) -> String {
    content
        .iter()
        .filter_map(MessageContent::as_thinking)
        .map(|thinking| thinking.thinking.as_str())
        .collect()
}

fn joined_text(content: &[MessageContent]) -> String {
    content.iter().filter_map(MessageContent::as_text).collect()
}

/// Remove the parts `is_part` matches from the streamed run that starts at
/// `run_start`, and drop any message there that held nothing else.
fn strip_streamed_parts(
    pending: &mut Vec<Message>,
    run_start: usize,
    is_part: fn(&MessageContent) -> bool,
) {
    let run = pending.split_off(run_start);
    pending.extend(run.into_iter().filter_map(|mut message| {
        let held = message.content.len();
        message.content.retain(|content| !is_part(content));
        (held == 0 || !message.content.is_empty()).then_some(message)
    }));
}

/// The message that carries one parsed tool call into history: the response's
/// reasoning, then `lead_text` (only the first call is given any), then the
/// call.
///
/// Every call gets its own copy of the reasoning. DeepSeek and Kimi want
/// `reasoning_content` on each assistant message with tool calls, and
/// `merge_split_tool_call_messages` (formats/openai.rs) rejoins the split
/// calls into one assistant message by matching it. The text goes on the
/// first call because that merge only folds in later messages with no content
/// of their own.
fn tool_call_history_message(
    request: &ToolRequest,
    reasoning: &[MessageContent],
    lead_text: Vec<MessageContent>,
) -> Message {
    let mut message = Message::assistant().with_generated_id();
    message.content.extend(reasoning.iter().cloned());
    message.content.extend(lead_text);
    message.with_tool_request_with_metadata(
        request.id.clone(),
        request.tool_call.clone(),
        request.metadata.as_ref(),
        request.tool_meta.clone(),
    )
}

/// What a tool-calling `response` adds to history, given `pending`, the turn's
/// unsaved messages (`messages_to_add`).
///
/// The reasoning is kept ONCE per tool-call message. It used to be kept up to
/// three times on the first one: an OpenAI-compatible stream sends each
/// reasoning delta (pushed and folded into one message as it arrives) and then
/// the whole reasoning again on the tool-call message, and the loop also saved
/// that replay as a message of its own before the per-call copies. Coalescing
/// the turn folded all three into one `Thinking`, so the first call's
/// `reasoning_content` no longer matched the others: the parallel calls went
/// out split across assistant messages, and DeepSeek/Kimi were re-sent the
/// extra copies on every later request. Now the streamed copy is removed when
/// the response repeats it (the replay equals the streamed reasoning or
/// extends it), and reasoning the stream did not send is left alone.
///
/// Text that came in the same message as the calls (a non-streamed reply, a
/// Google chunk, Bedrock) used to be dropped. It now goes on the first call,
/// unless it repeats text already streamed, as Ollama's XML tool-call prefix
/// does; text that extends what was streamed replaces the streamed part.
pub(crate) fn tool_call_history(
    pending: &mut Vec<Message>,
    response: &Message,
    requests: &[&ToolRequest],
) -> ToolCallHistory {
    let run_start = pending
        .iter()
        .rposition(|message| !is_streamed_ahead_of_tool_calls(message))
        .map_or(0, |index| index + 1);

    let mut reasoning: Vec<MessageContent> = response
        .content
        .iter()
        .filter(|content| matches!(content, MessageContent::Thinking(_)))
        .cloned()
        .collect();
    let replayed_reasoning = joined_thinking(&reasoning);
    let streamed_reasoning: String = pending[run_start..]
        .iter()
        .map(|message| joined_thinking(&message.content))
        .collect();
    if !streamed_reasoning.is_empty() && replayed_reasoning.starts_with(&streamed_reasoning) {
        // A replay without the signature the streamed deltas carried keeps it.
        let streamed_signature = pending[run_start..]
            .iter()
            .flat_map(|message| &message.content)
            .filter_map(MessageContent::as_thinking)
            .map(|thinking| thinking.signature.clone())
            .find(|signature| !signature.is_empty());
        if let (Some(signature), Some(MessageContent::Thinking(first))) =
            (streamed_signature, reasoning.first_mut())
        {
            if first.signature.is_empty() {
                first.signature = signature;
            }
        }
        strip_streamed_parts(pending, run_start, |content| {
            matches!(content, MessageContent::Thinking(_))
        });
    }

    let mut text: Vec<MessageContent> = response
        .content
        .iter()
        .filter(|content| matches!(content, MessageContent::Text(_)))
        .cloned()
        .collect();
    let same_message_text = joined_text(&text);
    let streamed_text: String = pending[run_start..]
        .iter()
        .map(|message| joined_text(&message.content))
        .collect();
    let (same_message_text, streamed_text) = (same_message_text.trim(), streamed_text.trim());
    if same_message_text.is_empty() {
        text.clear();
    } else if !streamed_text.is_empty() {
        if streamed_text.starts_with(same_message_text) {
            text.clear();
        } else if same_message_text.starts_with(streamed_text) {
            strip_streamed_parts(pending, run_start, |content| {
                matches!(content, MessageContent::Text(_))
            });
        }
    }

    let mut lead_text = Some(text);
    let mut calls = Vec::with_capacity(requests.len());
    for request in requests {
        calls.push(if request.tool_call.is_ok() {
            Some(tool_call_history_message(
                request,
                &reasoning,
                lead_text.take().unwrap_or_default(),
            ))
        } else {
            None
        });
    }
    // `lead_text` is still here only when no call parsed.
    let lead = lead_text.and_then(|text| {
        let content: Vec<MessageContent> = reasoning.iter().cloned().chain(text).collect();
        (!content.is_empty()).then(|| {
            Message::new(response.role.clone(), response.created, content).with_generated_id()
        })
    });

    ToolCallHistory { lead, calls }
}

/// Lane R19a (harness loop 2026-09-14, M19 DeepSeek forensics). Should a
/// pre-first-byte provider stall be retried, or has this turn already spent
/// its one retry?
///
/// `stream_committed` guards against retrying a stream that already
/// delivered content: once part of an answer has reached the user there is
/// no longer a "same request" that is safe to resend silently underneath it.
/// `already_retried` bounds the retry to exactly one attempt per turn — a
/// second stall falls through to the ordinary give-up path instead of
/// retrying forever.
///
/// A pure, free function on purpose: the decision is exercised directly by
/// tests below without needing to drive the whole streaming reply generator.
fn should_retry_first_byte_stall(
    provider_err: &ProviderError,
    stream_committed: bool,
    already_retried: bool,
) -> bool {
    provider_err.is_first_byte_provider_stall() && !stream_committed && !already_retried
}

/// Lane R19c (G24 HIGH-1 + HIGH-2). Retry a first-byte-stalled provider
/// stream exactly once, honestly against the cost reservation and bounded
/// against the turn's own first-byte ceiling.
///
/// **HIGH-1 (double-spend).** The pre-R19c retry called
/// `with_reservation_attempts(manager, invocation_id, attempts_reserved,
/// retry)` — a FRESH `AttemptAuthorization` scope that re-grants the
/// invocation's WHOLE `attempts_reserved` budget again, on top of whatever
/// the first stream already spent, without ever calling
/// `extend_reservation`. This instead extends the SAME reservation by
/// exactly one physical attempt (`extend_reservation(invocation_id, 1)`,
/// the real ledger call `authorized_attempts::authorize_attempt` itself uses
/// to extend — see that module's doc) and authorizes only that one attempt
/// locally (`with_authorized_attempts(1, …)`, no further extension capability
/// installed): if the retry's OWN internal transport-level retry logic wants
/// more than the one attempt just paid for, it is correctly refused rather
/// than silently re-granted a full fresh batch. `attempts_reserved: None`
/// (no paid hold on this call at all) skips the ledger entirely and retries
/// directly, matching how the ORIGINAL call ran when it took no hold.
///
/// **HIGH-2 (worst-case wall clock).** The retry is scoped under
/// `stream_budget::FIRST_BYTE_WINDOW_OVERRIDE` with whatever remains of
/// `DEFAULT_FIRST_BYTE_SECS` since `first_byte_wait_started` — NOT a fresh
/// window — so one coordinator turn spends at most
/// `DEFAULT_FIRST_BYTE_SECS` (~180s) of silent pre-first-byte waiting in
/// total across both attempts, not ~2x that. A stall that consumed the
/// whole ceiling on the first attempt leaves the retry almost no additional
/// silent wait — if the provider answers essentially immediately this time,
/// the retry still succeeds; if not, it fails fast instead of burning
/// another full window.
async fn retry_first_byte_stall(
    session_manager: &Arc<crate::session::SessionManager>,
    invocation: &crate::agents::reply_parts::ProviderInvocationContext,
    first_byte_wait_started: std::time::Instant,
    retry: impl std::future::Future<Output = Result<crate::providers::base::MessageStream, ProviderError>>
        + Send,
) -> Result<crate::providers::base::MessageStream, ProviderError> {
    let remaining =
        std::time::Duration::from_secs(crate::providers::stream_budget::DEFAULT_FIRST_BYTE_SECS)
            .saturating_sub(first_byte_wait_started.elapsed());
    // Lane R19d: `Secs(remaining)`, never `Unlimited` — the retry's own
    // ceiling is always a concrete (possibly zero) duration, not "no
    // first-byte timer at all".
    let scoped_retry = crate::providers::stream_budget::FIRST_BYTE_WINDOW_OVERRIDE.scope(
        crate::providers::stream_budget::FirstByteWindow::Secs(remaining),
        retry,
    );

    if invocation.attempts_reserved.is_none() {
        // No paid hold governs this call at all — retry with no reservation
        // scope, exactly like the original call.
        return scoped_retry.await;
    }
    match session_manager
        .extend_reservation(&invocation.invocation_id, 1)
        .await
    {
        Ok(crate::session::CostReservationOutcome::Granted { .. }) => {
            crate::cost_router::authorized_attempts::with_authorized_attempts(1, scoped_retry).await
        }
        Ok(other) => Err(ProviderError::ExecutionError(format!(
            "the cost reservation for this call would not authorize a retry attempt: {other:?}"
        ))),
        Err(extend_err) => Err(ProviderError::ExecutionError(format!(
            "could not extend the reservation for a retry attempt: {extend_err}"
        ))),
    }
}

// ── Y-poll-suspend: stop paying provider turns to watch unchanged workers ──
//
// Harness loop, 2026-09-21. A traced coordinator run took 38 turns, ~25 of
// them `view_session` / `goal_status` / `list_sessions`, wrote zero files,
// and grew its own context from 26k to 53k tokens — every one of those
// look-again turns re-sent the whole growing transcript as a fresh provider
// call. Across the cost ledger for every run since 2026-09-01, 56% of all
// spend ($43.20 of $76.89) went to turns that produced under 300 output
// tokens: input:output of 90:1, the shape of a coordinator looking, not
// working.
//
// The fix is not a smarter prompt: the repo already has host-side machinery
// — `drain_dispatched_goals` (`platform_extensions::orchestrator`), the same
// poller `CliSession::drain_dispatched_goals` uses after a headless run ends
// to wait out goals it dispatched — that tracks a dispatched worker's state
// changes, completion, and redispatch with zero LLM turns, one cheap SQLite
// read per descendant per poll tick. `DispatchPollGuard` below is what
// decides WHEN to hand the coordinator's turn to that poller instead of
// letting the model call another inspection tool: at most one inspection
// turn is free after a dispatch (or after any state change), because a
// coordinator that had its tool calls taken away entirely would be blind,
// not efficient — but a SECOND consecutive turn whose only tool calls read
// the exact same unchanged state is the measured waste above, verbatim.

/// Default bound on a single mid-turn suspension, overridable through
/// [`MID_TURN_DISPATCH_SUSPEND_SECS_KEY`].
///
/// QA 2026-09-25: this used to read `PERMAGENT_HEADLESS_DRAIN_SECS`, the
/// post-run drain's key, on the theory that both bound the same call. They
/// bound different things. The post-run cap is how long an exiting process
/// waits before it parks what is left; this cap is only how long the
/// coordinator sleeps before it gets a fresh look — the suspension runs the
/// drain with `DrainCapPolicy::ReturnOnExpiry`, so reaching it parks nothing.
/// Tuning one (the post-run key was raised to 900s as a stopgap for exactly
/// this bug) must not silently retune the other.
const DEFAULT_MID_TURN_DISPATCH_SUSPEND_CAP_SECS: u64 = 240;

/// Config key for [`DEFAULT_MID_TURN_DISPATCH_SUSPEND_CAP_SECS`].
const MID_TURN_DISPATCH_SUSPEND_SECS_KEY: &str = "PERMAGENT_MID_TURN_SUSPEND_SECS";

/// Read-only tools whose only effect is looking at existing state — never
/// dispatching, steering, escalating, or otherwise changing anything. Matched
/// against the tool's BASE name (the part after `extension_manager`'s own
/// `"{extension}__{tool}"` prefix, stripped the same way
/// `extension_manager::simple_tool_name`-style call sites do via
/// `rsplit("__")`), so this list stays correct regardless of which extension
/// namespace the orchestrator tools are registered under.
///
/// Deliberately narrow: `goal_advance`, `steer_goal`, `message_goal`,
/// `start_agent`, `send_message`, `create_roadmap`, `escalate`, and every
/// other tool that can change a goal's or a session's state are NOT here,
/// and a turn that calls any of them is never suspended by this guard even
/// if it also happens to call one of these.
const INSPECTION_ONLY_TOOLS: &[&str] = &[
    "list_sessions",
    "view_session",
    "view_attachment",
    "list_workers",
    "check_worker",
    "goal_status",
    "goal_list",
    "routing_roster",
];

fn is_inspection_only_tool(tool_name: &str) -> bool {
    let base = tool_name.rsplit("__").next().unwrap_or(tool_name);
    INSPECTION_ONLY_TOOLS.contains(&base)
}

/// Per-reply-loop memory for the suspension guard: how many consecutive
/// completed turns looked at the SAME dispatched-worker state with nothing
/// but inspection tools. Lives on the stack inside `reply_internal`'s
/// `async_stream!` — one instance per reply, never persisted and never
/// shared across sessions or turns of a different reply.
#[derive(Default)]
struct DispatchPollGuard {
    last_snapshot: Option<std::collections::BTreeMap<String, crate::goal_state::GoalState>>,
    /// How many consecutive observed turns have been inspection-only AND seen
    /// the identical snapshot as the turn before them. `1` is the one free
    /// look this rule allows (after a dispatch, or right after any state
    /// change); this needs to reach `2` before `observe_turn` says suspend.
    inspection_turns_since_change: u32,
}

impl DispatchPollGuard {
    /// Called once per completed turn. `tool_names` is every tool this turn
    /// called (base names); `snapshot` is the freshly-read dispatched-worker
    /// state for this coordinator, or `None` when nothing is dispatched (or
    /// the read itself failed) — either way there is no worker state to be
    /// blind about, so the guard stands down and forgets its own history
    /// rather than carry a stale comparison into whatever gets dispatched
    /// next.
    ///
    /// Returns `true` exactly when the loop should suspend instead of
    /// sending the next provider turn: this turn, and the one immediately
    /// before it, were both inspection-only against the same unchanged
    /// state.
    fn observe_turn(
        &mut self,
        tool_names: &[String],
        snapshot: Option<std::collections::BTreeMap<String, crate::goal_state::GoalState>>,
    ) -> bool {
        let Some(snapshot) = snapshot else {
            self.last_snapshot = None;
            self.inspection_turns_since_change = 0;
            return false;
        };
        let inspection_only =
            !tool_names.is_empty() && tool_names.iter().all(|name| is_inspection_only_tool(name));
        if !inspection_only {
            // An actionable turn — a dispatch, an escalation, a steer, a
            // message, or anything off the read-only allowlist — is progress,
            // not polling. Whatever it just changed makes the NEXT look a
            // fresh one either way, so the counter starts over rather than
            // carrying a stale "already looked once" into it.
            self.last_snapshot = Some(snapshot);
            self.inspection_turns_since_change = 0;
            return false;
        }
        let unchanged = self.last_snapshot.as_ref() == Some(&snapshot);
        self.last_snapshot = Some(snapshot);
        if unchanged {
            self.inspection_turns_since_change += 1;
        } else {
            // The model just learned something new since its last look — the
            // ONE free inspection this rule grants, not the first of a pair
            // of no-op turns.
            self.inspection_turns_since_change = 1;
        }
        self.inspection_turns_since_change > 1
    }

    /// Called after a suspension hands control back to the model with a
    /// compact state-change notice: that notice IS new information, so the
    /// next inspection of it is a fresh first look, not a continuation of
    /// whatever run of no-op turns triggered the suspension.
    fn record_resume(&mut self) {
        self.last_snapshot = None;
        self.inspection_turns_since_change = 0;
    }
}

/// Suspend the reply loop on `drain_dispatched_goals` — the SAME host poller
/// `CliSession::drain_dispatched_goals` uses after a headless run ends —
/// instead of returning control to the model for another look-again turn.
/// Costs one SQLite read per dispatched descendant per poll tick and zero
/// provider turns for as long as nothing changes; already knows how to
/// track completion, redispatch dependents, and park a stalled worker
/// without any of that needing to come back through this coordinator's own
/// turns.
///
/// Runs the drain with `DrainCapPolicy::ReturnOnExpiry`: reaching `cap`
/// ends the wait and nothing else — a worker still running is reported as
/// still running and left alone (QA 2026-09-25: the shared cap-expiry park
/// pass used to park an actively working worker here, and its uncommitted
/// edits went with it). A genuinely finished attempt the goal never
/// resolved (terminal receipt, non-terminal card) is still parked, exactly
/// as before.
///
/// Races the wait against `cancel_token` (when one is provided) so a cancel
/// arriving mid-suspension is not swallowed by an in-flight drain: parking
/// still-running goals at process exit is `CliSession::drain_dispatched_goals`'s
/// job at shutdown, not this mid-session guard's, so on cancellation this
/// simply stops waiting and returns `None` — the caller's existing
/// cancellation check at the top of the reply loop ends the turn exactly the
/// way it always does, with no special-cased cleanup added here.
///
/// Returns `None` on cancellation, or on the (deliberately rare —
/// `CoordinatorDrainGuard` in `orchestrator.rs`) case where another drain is
/// already running for this same coordinator: falling through to an ordinary
/// turn beats deadlocking on a wait with no way to know when the other one
/// will return. Otherwise returns a compact, human-readable summary of what
/// changed — never a transcript dump — built only from the state
/// transitions this wait actually observed plus the drain's own summary
/// counts.
async fn suspend_for_dispatched_goals(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    coordinator_session_id: &str,
    cap: std::time::Duration,
    cancel_token: &Option<CancellationToken>,
) -> Option<String> {
    let poll_interval = std::time::Duration::from_secs(2).min(cap);
    let transitions: Arc<std::sync::Mutex<Vec<(String, crate::goal_state::GoalState)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let transitions_for_closure = Arc::clone(&transitions);
    let drain = crate::agents::platform_extensions::orchestrator::drain_dispatched_goals(
        pool,
        coordinator_session_id,
        cap,
        poll_interval,
        crate::agents::platform_extensions::orchestrator::DrainCapPolicy::ReturnOnExpiry,
        move |session_id, state| {
            if let Ok(mut observed) = transitions_for_closure.lock() {
                observed.push((session_id.to_string(), state));
            }
        },
    );
    tokio::pin!(drain);

    let result = match cancel_token {
        Some(token) => {
            tokio::select! {
                biased;
                _ = token.cancelled() => return None,
                result = &mut drain => result,
            }
        }
        None => drain.await,
    };

    let observed = transitions.lock().ok()?.clone();
    match result {
        Ok(summary) => Some(format_compact_resume_notice(&observed, summary)),
        Err(error) => {
            tracing::warn!(
                target: "permagentd::brain",
                session_id = %coordinator_session_id,
                %error,
                "mid-turn dispatch-poll suspension could not read dispatched-goal state; \
                 resuming the model turn rather than staying suspended against state it cannot \
                 read anyway"
            );
            Some(format!(
                "(Dispatched worker status could not be read while this turn was suspended: \
                 {error}. Resuming.)"
            ))
        }
    }
}

/// Render what `suspend_for_dispatched_goals` observed as one compact
/// message — the half of the fix that is not the suspension itself: what
/// comes back to the model must be a state-change summary, never the
/// transcript the host poller generated while producing it.
fn format_compact_resume_notice(
    transitions: &[(String, crate::goal_state::GoalState)],
    summary: crate::agents::platform_extensions::orchestrator::DrainSummary,
) -> String {
    let mut lines = vec![
        "(Resuming — a dispatched worker's status changed while this turn was suspended; \
         no provider turns were spent waiting.)"
            .to_string(),
    ];
    if transitions.is_empty() {
        lines.push("  no further state changes observed before the wait cap".to_string());
    } else {
        for (session_id, state) in transitions {
            lines.push(format!("  worker {session_id}: {}", state.binding()));
        }
    }
    // QA 2026-09-25: this wait parks nothing at its cap (`ReturnOnExpiry`),
    // so never claim it did. `timed_out` here is only a genuine stall park.
    let mut tally = format!(
        "{} finished, {} still running",
        summary.finished, summary.still_running
    );
    if summary.timed_out > 0 {
        tally.push_str(&format!(
            ", {} parked as stalled (attempt over, or idle past its budget)",
            summary.timed_out
        ));
    }
    tally.push('.');
    lines.push(tally);
    lines.join("\n")
}

async fn persist_turn_ending_message(
    session_manager: &Arc<crate::session::SessionManager>,
    session_id: &str,
    message: &Message,
) {
    if let Err(e) = session_manager.add_message(session_id, message).await {
        warn!("Failed to persist the turn-ending provider message: {e}");
    }
}

/// Save what this turn already streamed, THEN the turn-ending notice.
///
/// A transient error after text reached the user (a body cut off, a socket
/// reset, an SSE `error` chunk mid-answer) ends the turn with a notice that
/// asks for a resend. That notice was saved at once, while the half answer
/// above it on screen was still queued in `streamed` and only saved at the end
/// of the iteration, so the saved transcript read [notice, half answer]. The
/// desktop reloads the transcript when a turn ends, so the "stream dropped"
/// line jumped ABOVE the answer it explains and the session ended on a
/// truncated reply with nothing under it: the wording QA could not confirm on
/// 2026-09-19. Saving the streamed part first keeps the saved order the order
/// the user saw. The same coalescing as the end-of-iteration save, so a
/// reasoning item and its text stay one message.
async fn persist_streamed_then_notice(
    session_manager: &Arc<crate::session::SessionManager>,
    session_id: &str,
    streamed: &mut Conversation,
    conversation: &mut Conversation,
    notice: &Message,
) {
    if !streamed.is_empty() {
        let (coalesced, _) = coalesce_turn_messages(streamed.messages().clone());
        for message in &coalesced {
            if let Err(e) = session_manager.add_message(session_id, message).await {
                warn!("Failed to persist the text streamed before a turn-ending error: {e}");
            }
        }
        conversation.extend(coalesced);
        *streamed = Conversation::default();
    }
    persist_turn_ending_message(session_manager, session_id, notice).await;
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
            after_turn_manager: Self::create_after_turn_manager(),
            tool_inspection_manager: Self::create_tool_inspection_manager(
                permission_manager,
                provider.clone(),
                session_manager_for_inspectors,
            ),
            container: Mutex::new(None),
            last_extension_config_mtime: Mutex::new(None),
            last_context_length_failure: Mutex::new(None),
            transient_failover: Mutex::new(None),
            #[cfg(test)]
            test_failover: Mutex::new(None),
            #[cfg(test)]
            test_failover_request_tokens: Mutex::new(Vec::new()),
        }
    }

    /// Create a tool inspection manager with default inspectors
    /// The after-turn chain. `PrematureDoneGuard` is the first hook: it refuses
    /// to let a turn that CHANGED CODE end without a passing verify, reusing
    /// `cost_router::decide_hold` — the same pure decision goal dispatch makes.
    /// Safe on ordinary conversation because the guard only consults that
    /// decision when the session actually wrote something.
    ///
    /// `ReviewerMandate` runs second: it refuses to let a turn that changed
    /// files end without an independent, cross-family reviewer having been
    /// asked (or a `Park` explaining why none could be). Same applicability
    /// gate as `PrematureDoneGuard` — ordinary conversation never triggers it.
    ///
    /// `RoadmapDispatchGuard` runs third (X-dag-2, coding-harness excellence
    /// iteration): it refuses to let a turn end claiming a roadmap's goals
    /// are dispatched/in-progress when the durable goal-card state says
    /// otherwise — Lane M3's Defect M3-2. Its own applicability gate is
    /// durable state, not the transcript: a session that never called
    /// `create_roadmap` has no cards attributed to it, so ordinary
    /// conversation and non-harness sessions never trigger it either.
    ///
    /// `NoRoadmapGuard` runs fourth (Lane Y5, harness loop, from Lane A13's
    /// M13 forensics): it refuses to let a coding-harness COORDINATOR turn end
    /// having never called `decompose_roadmap`/`create_roadmap` at all — the
    /// fx02 "prose roadmap, say the word and I'll dispatch" and fx06 "denied
    /// shell, then a question" exits, which `PrematureDoneGuard` (needs a
    /// successful mutation) and `RoadmapDispatchGuard` (needs cards or an
    /// approved proposal) both structurally cannot see. Its gate is the same
    /// predicate `CodingDagInspector` uses, so no other recipe and no
    /// dispatched worker can reach it, and it fires at most once per session.
    fn create_after_turn_manager() -> crate::after_turn::AfterTurnManager {
        let mut manager = crate::after_turn::AfterTurnManager::new();
        manager.add_hook(Box::new(crate::after_turn::PrematureDoneGuard::new()));
        manager.add_hook(Box::new(crate::after_turn::ReviewerMandate::new()));
        manager.add_hook(Box::new(crate::after_turn::RoadmapDispatchGuard::new()));
        manager.add_hook(Box::new(crate::after_turn::NoRoadmapGuard::new()));
        // Lane R35a (A32 root cause 1): refuses to let the turn end in prose
        // while a goal already carries a worker's verified, committed PASS
        // (`ready`/`review` + `dispatch_evidence.verdict.status == "pass"`)
        // that nothing has advanced to Review/land yet. Complements
        // `NoRoadmapGuard`'s own stranded-Ready nudge (R31b), which fires on
        // a different shape (resumed, not verified).
        manager.add_hook(Box::new(
            crate::after_turn::VerifiedGoalAwaitingLandGuard::new(),
        ));
        // Lane R18f: headless-only backstop for the rule-5 free-text ending
        // NoRoadmapGuard cannot see (a roadmap tool call that was ATTEMPTED
        // but failed, e.g. -32002, reads to NoRoadmapGuard as "rule 1
        // satisfied"). See Rule5HonestFailureGuard's doc comment.
        manager.add_hook(Box::new(crate::after_turn::Rule5HonestFailureGuard::new()));
        manager
    }

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

        // Coding harness roots are planners/orchestrators. They may inspect,
        // research, and build the graph, but mutation belongs to a dispatched
        // DAG node so verification and approval cannot be bypassed.
        tool_inspection_manager.add_inspector(Box::new(
            crate::tool_inspection::CodingDagInspector::new(session_manager.clone()),
        ));

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

    /// The reason an INSPECTOR denied `request_id`, if one did.
    ///
    /// A ProgressMonitor block (L1/L3) carries an actionable "try a different
    /// approach" reason. Surface it to the model in place of the generic
    /// declined text so it can self-correct — the whole point of the L1 nudge.
    ///
    /// Y4c (harness loop, 2026-09-10, from Lane A12's M12 forensics): this
    /// used to be the ProgressMonitor's reason and NOTHING else, so every
    /// other inspector's carefully written reason was replaced by
    /// `DECLINED_RESPONSE` — text that tells the model *the user* refused.
    /// `CodingDagInspector`'s reason is the case A12 measured: it explains
    /// that a top-level coding session must dispatch a worker rather than
    /// edit, and the coordinator never saw it. fx09 msg 18 reasons "The user
    /// declined both shell and write tool calls"; fx04 msg 32 opens "I
    /// understand - you've stopped this implementation". Between fx04, fx05
    /// and fx09 that cost 11 tool calls and about a third of each run, on
    /// denials whose reason was sitting in `inspection_results` the whole
    /// time. A denial with no inspector reason behind it — an actual user
    /// decline — still gets `DECLINED_RESPONSE`, unchanged.
    ///
    /// The ProgressMonitor keeps first claim when several inspectors deny the
    /// same call, so its behavior is exactly what it was.
    ///
    /// G11 (harness loop, 2026-09-10): unlike the fixed/templated reasons
    /// above, `AdversaryInspector`'s is a live LLM completion over the tool
    /// call's own (possibly attacker-controlled) arguments — it can echo
    /// injected text back into context or act as a bypass-iteration oracle.
    /// Inspectors listed here keep the generic `DECLINED_RESPONSE` instead.
    const NON_COACHING_INSPECTORS: &[&str] = &["adversary"];

    fn inspector_deny_reason(
        request_id: &str,
        inspection_results: &[crate::tool_inspection::InspectionResult],
    ) -> Option<String> {
        let mut first_other: Option<&crate::tool_inspection::InspectionResult> = None;
        for result in inspection_results.iter().filter(|r| {
            r.tool_request_id == request_id
                && r.action == crate::tool_inspection::InspectionAction::Deny
                && !r.reason.trim().is_empty()
                && !Self::NON_COACHING_INSPECTORS.contains(&r.inspector_name.as_str())
        }) {
            if result.inspector_name == crate::tool_monitor::PROGRESS_MONITOR_NAME {
                return Some(result.reason.clone());
            }
            if first_other.is_none() {
                first_other = Some(result);
            }
        }
        first_other.map(|r| r.reason.clone())
    }

    fn handle_denied_tools(
        permission_check_result: &PermissionCheckResult,
        request_to_response_map: &mut HashMap<String, Message>,
        inspection_results: &[crate::tool_inspection::InspectionResult],
    ) {
        for request in &permission_check_result.denied {
            if let Some(response) = request_to_response_map.get_mut(&request.id) {
                let text = Self::inspector_deny_reason(&request.id, inspection_results)
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
        if self.config.headless_coding_coordinator
            && !headless_coding_coordinator_allows_tool(&tool_call.name)
        {
            return (
                request_id,
                Err(ErrorData::new(
                    ErrorCode::RESOURCE_NOT_FOUND,
                    format!(
                        "Tool '{}' is unavailable to the headless coding coordinator",
                        tool_call.name
                    ),
                    None,
                )),
            );
        }
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
            crate::session::run_timing::record_approved_dispatch(&session.id);
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
            crate::session::run_timing::record_approved_dispatch(&session.id);
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
                crate::session::run_timing::record_approved_dispatch(&session.id);
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
        // Owned copies for the offload stub's file path — `request_id` is still
        // needed by this function's own return value.
        let offload_tool_name = tool_name_str.clone();
        let offload_request_id = request_id.clone();
        let offload_session_id = session.id.clone();
        // Split the stream off before the future moves into the async block.
        let ToolCallResult {
            notification_stream,
            result: result_future,
        } = result;
        let resolved = async move {
            let out = result_future.await;
            // Always-on: every resolved tool result passes here, so an
            // oversized one is spilled to a file and replaced by a head+tail
            // stub carrying its path (see `large_response_handler`).
            let out = super::large_response_handler::process_tool_response(
                out,
                &offload_tool_name,
                &offload_request_id,
                &offload_session_id,
            );

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

        // Guided-tour lesson loader — always available (read-only, no deps),
        // EXCEPT for a compact-profile dispatched goal worker (R48c, harness
        // loop "Lane R48c"): this tool is appended here unconditionally,
        // outside the extension-grant mechanism `GOAL_WORKER_EXTENSION_KEYS`
        // narrows, so it survived R30a's own narrowing and A39's compact-
        // profile-motivating measurement (`platform__load_feature_lesson`,
        // 1 of the 12 tools measured).
        if (extension_name.is_none() || extension_name.as_deref() == Some("platform"))
            && !self.config.compact_worker_profile
        {
            prefixed_tools.push(platform_tools::load_feature_lesson_tool());
        }

        if extension_name.is_none() {
            if let Some(final_output_tool) = self.final_output_tool.lock().await.as_ref() {
                prefixed_tools.push(final_output_tool.tool());
            }
        }

        if self.config.headless_coding_coordinator {
            prefixed_tools.retain(|tool| headless_coding_coordinator_allows_tool(&tool.name));
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

    /// R46a: record the raw provider-error text behind a context-length
    /// give-up, for [`Agent::take_context_length_failure`] to read after the
    /// reply stream ends. Called from exactly one place in the reply loop
    /// (the `ProviderError::ContextLengthExceeded` arm's final, no-more-
    /// compaction-attempts branch) — see that field's doc comment for why.
    async fn record_context_length_failure(&self, detail: String) {
        *self.last_context_length_failure.lock().await = Some(detail);
    }

    /// Takes (clears) the most recent context-length-exceeded detail
    /// recorded by [`Agent::record_context_length_failure`], if any. See
    /// `last_context_length_failure`'s doc comment: this is a non-user-
    /// facing side channel a headless caller (a `delegate`/`delegate_many`
    /// subagent) opts into reading to classify an `AgentRuntimeOutcome::
    /// Failed` instead of reporting a bare "Subagent runtime failed".
    pub async fn take_context_length_failure(&self) -> Option<String> {
        self.last_context_length_failure.lock().await.take()
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
        self.reply_with_recognition(user_message, session_config, cancel_token, None)
            .await
    }

    /// Reply entry point carrying the optional existing recognition retrieval
    /// identity for durable provider-attribution write-back. Existing callers
    /// should continue using [`Self::reply`]; this additive seam is used only
    /// by the recall-aware server route.
    #[instrument(
        skip(self, user_message, session_config, cancel_token, recognition_retrieval_id),
        fields(user_message, trace_input, session.id = %session_config.id)
    )]
    pub async fn reply_with_recognition(
        &self,
        user_message: Message,
        session_config: SessionConfig,
        cancel_token: Option<CancellationToken>,
        recognition_retrieval_id: Option<String>,
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

        // One top-level interactive Agent::reply is one user task. The ID is
        // durable session state, so after-turn continuations, compaction, and
        // provider retries remain part of this same budget identity. Child and
        // headless sessions inherit their parent's ID at session creation.
        let session_type = session_manager
            .get_session(&session_config.id, false)
            .await?
            .session_type;
        if starts_new_budget_task(session_type, &message_text) {
            session_manager
                .begin_budget_task_with_contract(&session_config.id, &user_message)
                .await?;
        }

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

                let compact_provider = self.provider().await?;
                let compact_accounting = self.accounted_fast_completion(
                    &session,
                    Arc::clone(&compact_provider),
                    true,
                );
                match compact_messages_accounted(
                    &session_config.id,
                    &conversation_to_compact,
                    false,
                    compact_accounting.as_ref(),
                )
                .await
                {
                    Ok((compacted_conversation, summarization_usage)) => {
                        session_manager.replace_conversation(&session_config.id, &compacted_conversation).await?;
                        // `compact_messages_accounted` already settled this
                        // exact invocation atomically with the session rollup.
                        // Keep the usage binding only as a compile-time guard
                        // against accidentally reverting to unaccounted calls.
                        let _ = summarization_usage;

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

            let mut reply_stream = self
                .reply_internal(
                    final_conversation,
                    session_config,
                    session,
                    cancel_token,
                    recognition_retrieval_id,
                )
                .await?;
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
        recognition_retrieval_id: Option<String>,
    ) -> Result<BoxStream<'_, Result<AgentEvent>>> {
        // A new message goes to the model the operator chose: a fallback taken
        // for a transient failure on an earlier reply ends here, whatever is
        // left of its time box. Before the prompt is built, which names the
        // live model.
        let restored_for_new_reply = self.end_transient_failover(&session.id, true).await;
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
        // Lane R62f (A61 miss 2): the turn-budget "git commit NOW" nudge is
        // for dispatched goal workers only, never a capped coordinator.
        let turn_budget_nudge_eligible = crate::after_turn::turn_budget_nudge_applies(
            session_config.max_turns,
            session.session_type,
        );
        let reply_stream_span = tracing::info_span!(target: "permagent::agents::agent", "reply_stream", session.id = %session_config.id);
        let inner = Box::pin(async_stream::try_stream! {
            // Working on entry; Drop records done/error on every exit path (#348).
            let mut state_guard = ReplyStateGuard::start();
            if let Some(notice) = restored_for_new_reply {
                yield AgentEvent::Message(notice);
            }
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
            // Consecutive turns that produced no text and no tool call, each
            // answered with a continuation — see `decide_empty_turn`.
            let mut empty_turn_continuations = 0u32;
            // Consecutive turns whose tool calls could not be parsed. A
            // malformed call used to END the session — the single most common
            // weak-model failure mode, terminating instead of being corrected.
            // It is now an observation the model can retry against, bounded so
            // a model that cannot recover still stops, and cleared by any turn
            // that parses (the same shape as `compaction_attempts` at the
            // provider round-trip and the S5 monologue counter below).
            let mut consecutive_parse_failure_turns = 0u32;
            // How many times the after-turn chain has already refused to let
            // THIS reply finish. Bounds the re-entry: a hook that keeps saying
            // "not yet" is eventually told to stop and hand over.
            let mut after_turn_holds: u8 = 0;
            // A PERMANENT provider rejection (billing / auth) gets exactly ONE
            // model swap per reply. Without this bound, a fallback that is also
            // out of credit would ping-pong the turn forever — the same
            // unbounded-retry defect this PR removes from the retry layer.
            let mut permanent_failure_fallback_used = false;
            // The provider this reply failed over FROM, set by every arm that
            // sets `permanent_failure_fallback_used`. The context-overflow arm
            // reads it to tell a stand-in's small window from a long history,
            // and switches the session back to it (see that arm).
            let mut failed_over_from: Option<Arc<dyn Provider>> = None;
            // Why it failed over, when that was a billing or auth rejection
            // (its payload-free summary). The overflow arm hands the session
            // back either way, but must not tell the user to "resend once it is
            // reachable" to a provider that rejects every request until its
            // credit or key is fixed: the resend would fail over to the same
            // stand-in and be refused again.
            let mut failed_over_permanent_cause: Option<String> = None;
            // Same-model retries spent on the CURRENT failing call (see
            // `cost_router::fallback::same_model_retry_delay`). Reset by any
            // iteration that was not itself such a retry, so a long harness
            // run gets a fresh budget for each separate blip.
            let mut same_model_retries: usize = 0;
            // The previous iteration only re-sent the same call after a
            // transient failure: it was not a new model turn, so it does not
            // count against `max_turns`.
            let mut resending_same_call = false;
            // R28a (HARNESS_LOOP_LOG.md "A27 analysis" misses 1/2): fires at
            // most once per reply, mirroring `monologue_nudged` above — see
            // `after_turn::turn_budget_nudge_text`'s own doc comment for why
            // this lives outside the `AfterTurn` chain.
            let mut turn_budget_nudged = false;
            // Y-poll-suspend (harness loop, 2026-09-21): tracks whether this
            // coordinator is looking at the SAME unchanged dispatched-worker
            // state turn after turn. Ledger evidence across every run since
            // 2026-09-01: 56% of all coordinator spend — $43.20 of $76.89 —
            // paid for turns producing under 300 output tokens, and one
            // traced run took 38 turns (~25 of them view_session/goal_status/
            // list_sessions), wrote zero files, and grew its own context
            // 26k -> 53k tokens, re-sent in full on every one of those
            // look-again turns. See `DispatchPollGuard` for the rule this
            // state enforces.
            let mut dispatch_poll_guard = DispatchPollGuard::default();
            // v60 (node A4): the invocation id of an iteration that failed and
            // made the loop re-run the turn (provider failover or recovery
            // compaction). The next dispatch is labelled a retry of it.
            let mut retry_of_failed_call: Option<String> = None;

            loop {
                if is_token_cancelled(&cancel_token) {
                    // Preserve the existing cancellation outcome rather than
                    // reporting the preceding failed attempt as this exit.
                    state_guard.begin_iteration();
                    break;
                }

                {
                    let guard = self.final_output_tool.lock().await;
                    if let Some(ref output) = guard.as_ref().and_then(|fot| fot.final_output.clone()) {
                        state_guard.begin_iteration();
                        yield AgentEvent::Message(Message::assistant().with_text(output));
                        break;
                    }
                }

                if !std::mem::take(&mut resending_same_call) {
                    turns_taken += 1;
                }
                if turns_taken > max_turns {
                    // Re-entry means the prior attempt still needed a tool
                    // follow-up or continuation; reaching this cap is not a
                    // completed reply even if that attempt's provider call
                    // itself succeeded.
                    state_guard.mark_error();
                    yield AgentEvent::Message(
                        Message::assistant().with_text(
                            "I've reached the maximum number of actions I can do without user input. Would you like me to continue?"
                        )
                    );
                    break;
                }

                // Clear the prior attempt's error only after the entry guards
                // allow a real next iteration. An after-turn continuation
                // stopped by max_turns has not recovered from that failure.
                state_guard.begin_iteration();

                // Lane X-lat (coding-harness excellence iteration): one
                // recorder per outer-loop turn attempt. `finish()` is called
                // at the very end of this loop body, after the after-turn
                // hooks and any context-management work, and the resulting
                // record is attached to this turn's already-settled
                // `cost_ledger` row via `update_cost_ledger_timing`.
                let mut turn_timing = TurnTimingRecorder::start();

                // A transient fallback whose time box is up goes back to the
                // model the session was on before this call is authorized, so
                // the call, its reservation and its prompt are all for that
                // model. The prompt is rebuilt: it names the live model.
                if let Some(notice) = self.end_transient_failover(&session_config.id, false).await {
                    permanent_failure_fallback_used = false;
                    failed_over_from = None;
                    failed_over_permanent_cause = None;
                    (tools, toolshim_tools, system_prompt) =
                        self.prepare_tools_and_prompt(&session_config.id, &session.working_dir).await?;
                    yield AgentEvent::Message(notice);
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

                // ContextPacket is a projection of the exact request inputs at
                // this seam. Keep unavailable partitions missing rather than
                // guessing (notably project memory and Spectral recall are
                // rendered into the prompt by other owners). Tool output is
                // recoverable from the conversation and tool definitions are
                // serialized exactly as the provider-facing values.
                let tool_schema = serde_json::to_string(&tools).ok();
                let tool_output = conversation_with_moim
                    .messages()
                    .iter()
                    .filter(|message| message.is_tool_response())
                    .map(Message::as_concat_text)
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                let context_contributions = self.prompt_manager.lock().await.context_contributions();
                let context_packet = crate::context_packet::assemble_with_contributions(
                    Some(system_prompt.stable_prefix()),
                    tool_schema.as_deref(),
                    &context_contributions,
                    (!tool_output.is_empty()).then_some(tool_output.as_str()),
                    &[],
                );
                debug!(
                    target: "permagent.context_packet",
                    session_id = %session_config.id,
                    packet = ?context_packet,
                    "coding model request context packet"
                );

                // Authorize this exact provider/model invocation before any
                // transport work begins. The captured identity is deliberately
                // independent of mutable session settings so a provider switch
                // on a later turn cannot rewrite this call's attribution.
                let turn_provider = self.provider().await?;
                let turn_model_name = turn_provider.get_model_config().model_name;
                // I1 (harness loop, 2026-09-10): reuse a still-pending,
                // TTL-valid reservation grant from a prior headless gate
                // auto-answer for THIS session, if one exists, instead of
                // always minting a fresh invocation_id — see
                // `reusable_gate_grant_invocation_id`'s own doc comment for
                // why a fresh id every call meant such a grant could never
                // be consumed (F10's forensics).
                //
                // F3 (harness loop, Round F, from Lane G9's HIGH finding):
                // `turn_provider`/`turn_model_name` are threaded in here so
                // a grant is only ever reused when it was minted for THIS
                // exact provider/model — a provider/role switch on a later
                // turn (this codebase's own routing/role-pin machinery does
                // this) now falls through to a fresh gate evaluation instead
                // of silently reusing a different provider's dollar bound.
                let invocation_id = match crate::agents::reply_parts::reusable_gate_grant_invocation_id(
                    &self.config.session_manager,
                    &session.id,
                    turn_provider.get_name(),
                    &turn_model_name,
                )
                .await
                {
                    Some(reused) => reused,
                    None => Uuid::new_v4().to_string(),
                };
                // v60 (node A4): why this call is being made, from what the
                // loop knows before dispatch (see `turn_call_provenance`).
                let call_provenance = crate::agents::reply_parts::turn_call_provenance(
                    conversation.messages(),
                    turns_taken,
                    retry_of_failed_call.take(),
                );
                // Sized to the request about to be sent (the same system
                // prompt, messages and tools handed to
                // `stream_response_from_provider` below), not the model's whole
                // context window: the harness's first call was refused with
                // "requested bound $1.56, ceiling $1.00" for a ~30k-token
                // prompt because this hold priced all 272k input tokens of
                // gpt-5.4-mini's window. An uncountable request (image/audio in
                // context) still falls back to the full window.
                let invocation = match self
                    .reserve_provider_invocation_for_request(
                        &session,
                        &turn_provider,
                        invocation_id.clone(),
                        &system_prompt,
                        conversation_with_moim.messages(),
                        &tools,
                        call_provenance.clone(),
                    )
                    .await
                {
                    Ok(invocation) => {
                        // Lane R18a-b (G21 HIGH-1): a per-run observation, not
                        // a compile-time literal — see
                        // `reply_parts::record_provider_call_authorized`'s doc
                        // comment. This arm never went through the
                        // gate/failure path at all: the reservation itself
                        // authorized the call, so it plainly proceeds WITH
                        // authorization.
                        crate::agents::reply_parts::record_provider_call_authorized(true);
                        invocation
                    }
                    Err(failure) => {
                        let outcome = self
                            .handle_provider_authorization_failure(
                                &session,
                                &invocation_id,
                                &failure,
                                turn_provider.get_name(),
                                &turn_model_name,
                            )
                            .await;
                        // Y4a (harness loop, 2026-09-10, from Lane A12's M12
                        // forensics): when the handler got the gate headlessly
                        // auto-answered it ALSO minted a `granted_via_gate=1`,
                        // `state='pending'` reservation under this very
                        // `invocation_id` — a paid-for authorization. Reusing
                        // that id here takes `reserve_provider_invocation`'s
                        // early idempotent lookup (`AlreadyReserved`), so the
                        // call goes out on THIS turn. Pre-Y4a the code always
                        // fell through to the message + `break` below, and the
                        // `reusable_gate_grant_invocation_id` seam above could
                        // only consume the grant on a LATER turn — which a
                        // coordinator never gets, because its turn ending is
                        // the run ending (A12: fx01 and fx09 each threw away a
                        // correct worker result this way; fx07 was truncated).
                        // Exactly one retry, never a loop: a second failure
                        // takes the unchanged park path.
                        let retried = if outcome.gate_grant_minted {
                            match self
                                .reserve_provider_invocation_for_request(
                                    &session,
                                    &turn_provider,
                                    invocation_id.clone(),
                                    &system_prompt,
                                    conversation_with_moim.messages(),
                                    &tools,
                                    call_provenance,
                                )
                                .await
                            {
                                Ok(invocation) => Some(invocation),
                                Err(retry_failure) => {
                                    tracing::warn!(
                                        session_id = %session_config.id,
                                        invocation_id = %invocation_id,
                                        "reservation gate was auto-granted but the single retry \
                                         still could not authorize the call ({retry_failure}); \
                                         ending the turn"
                                    );
                                    None
                                }
                            }
                        } else {
                            None
                        };
                        match retried {
                            Some(invocation) => {
                                // Lane R18a-b (G21 HIGH-1): reached only when
                                // `outcome.gate_grant_minted` was true AND the
                                // retried reservation itself succeeded — a
                                // real `ProviderAuthorizationOutcome` grant
                                // backs this call, never an assumption.
                                crate::agents::reply_parts::record_provider_call_authorized(true);
                                invocation
                            }
                            None => {
                                let reason = outcome.detail;
                                let message = Message::assistant().with_text(format!(
                                    "I did not send this request because provider spend could not be authorized: {reason}."
                                ));
                                persist_turn_ending_message(&session_manager, &session_config.id, &message)
                                    .await;
                                yield AgentEvent::Message(message);
                                state_guard.mark_error();
                                break;
                            }
                        }
                    }
                };

                turn_timing.start_model();
                // Lane R19c (G24 HIGH-2): marks when THIS turn started
                // waiting for a first byte, so a first-byte-stall retry can
                // be given the REMAINING part of `DEFAULT_FIRST_BYTE_SECS`
                // instead of a fresh window — see the retry arm below and
                // `stream_budget::FIRST_BYTE_WINDOW_OVERRIDE`'s doc comment.
                let first_byte_wait_started = std::time::Instant::now();
                // Carry the hold's count and extension authorizer into physical
                // retries. Exhausting the count requires a new one-attempt grant.
                // Unpaid work keeps the provider's retry policy unchanged.
                let (stream_result, attempt_evidence) = crate::cost_router::authorized_attempts::with_reservation_attempts_evidence(
                    Arc::clone(&session_manager),
                    invocation.invocation_id.clone(),
                    invocation.attempts_reserved,
                    Self::stream_response_from_provider(
                        turn_provider.clone(),
                        &session_config.id,
                        &system_prompt,
                        conversation_with_moim.messages(),
                        &tools,
                        &toolshim_tools,
                        invocation.reserved_output_tokens,
                    ),
                )
                .await;
                let mut stream = match stream_result {
                    Ok(stream) => stream,
                    Err(error) => {
                        // A typed local admission refusal can prove that no
                        // retry reached HTTP. Every other pre-stream error may
                        // have handed work to the provider and keeps its hold.
                        if let Some(reservation_id) = invocation.reservation_id.as_deref() {
                            crate::cost_router::authorized_attempts::finalize_missing_usage(
                                &session_manager,
                                reservation_id,
                                crate::cost_router::authorized_attempts::is_proven_not_sent(
                                    &error,
                                    &attempt_evidence,
                                ),
                            )
                            .await?;
                        }
                        let message = Message::assistant()
                            .with_text(format!("Ran into this error: {error}."));
                        persist_turn_ending_message(&session_manager, &session_config.id, &message)
                            .await;
                        yield AgentEvent::Message(message);
                        state_guard.mark_error();
                        break;
                    }
                };
                // A provider may attach the same cumulative usage snapshot to
                // several stream frames. Account once, using the final snapshot,
                // under one stable key for this logical invocation.
                let mut terminal_usage: Option<ProviderUsage> = None;

                let current_turn_tool_count = conversation.messages().iter()
                    .flat_map(|m| m.content.iter())
                    .filter(|c| matches!(c, MessageContent::ToolRequest(_)))
                    .count()
                    .saturating_sub(pre_turn_tool_count);

                let tool_pair_provider = self.provider().await?;
                let tool_pair_accounting = self.accounted_fast_completion(
                    &session,
                    Arc::clone(&tool_pair_provider),
                    false,
                );
                let tool_pair_summarization_task = maybe_summarize_tool_pairs_accounted(
                    tool_pair_provider,
                    session_config.id.clone(),
                    conversation.clone(),
                    tool_call_cut_off,
                    current_turn_tool_count,
                    tool_pair_accounting,
                );

                let mut no_tools_called = true;
                // What this turn actually produced, for `decide_empty_turn`: any
                // non-empty assistant text, any tool request (parsed or not),
                // and the provider's reason if it stopped the response early.
                let mut produced_text_this_turn = false;
                let mut requested_tool_this_turn = false;
                let mut incomplete_reason_this_turn: Option<String> = None;
                // One strike and one recovery observation per turn: a single
                // truncated response can carry several malformed calls, and
                // three near-identical "could not be parsed" messages in one
                // turn is noise the model pays for on every turn after.
                let mut saw_parse_failure_this_turn = false;
                // Y-poll-suspend: every tool base name (extension prefix
                // stripped, matching `extension_manager`'s own `rsplit("__")`
                // idiom) this turn actually called, across every streamed
                // response frame — `DispatchPollGuard::observe_turn` reads
                // this once the turn is over to tell "only looked" from "did
                // something".
                let mut tool_names_this_turn: Vec<String> = Vec::new();
                let mut messages_to_add = Conversation::default();
                let mut tools_updated = false;
                let mut did_recovery_compact_this_iteration = false;
                let mut did_switch_provider_this_iteration = false;
                // Set by the same-model retry arm: this call failed
                // transiently before anything reached the user, and the loop
                // re-sends it to the SAME model after a backoff.
                let mut did_retry_same_model_this_iteration = false;
                // Tokens/tools shown to the user this stream: after this flips,
                // a transient error must not silent-switch (the user already
                // saw a partial answer). Billing/auth still announce.
                let mut stream_committed = false;
                let mut rejected_without_usage = false;
                let mut exit_chat = false;
                // Lane R19a: a pre-first-byte provider stall gets exactly one
                // retry of the identical request. This bounds it to one per
                // turn attempt — see `should_retry_first_byte_stall`.
                let mut first_byte_stall_retried = false;
                // v60 (node A4): the response shape of THIS invocation, for
                // the ledger row's tool_call_count/completion_kind.
                let mut stream_outcome =
                    crate::session::session_manager::StreamOutcomeTracker::default();

                // Track whether this provider turn has already emitted visible
                // thinking so a later tool-call chunk can suppress replayed
                // reasoning without hiding final-only non-streaming thoughts.
                let mut surfaced_thinking_in_turn = false;

                while let Some(next) = stream.next().await {
                    turn_timing.record_first_token();
                    if is_token_cancelled(&cancel_token) || exit_chat {
                        break;
                    }

                    stream_outcome.observe_frame(next.is_err());
                    if let Ok((Some(ref response), _)) = next {
                        stream_outcome.observe_message(response);
                    }
                    if let Err(error) = &next {
                        rejected_without_usage = crate::cost_router::authorized_attempts::is_unbilled_stream_error(
                            error, &attempt_evidence, stream_committed, first_byte_stall_retried,
                        );
                    }
                    match next {
                        Ok((response, usage)) => {
                            compaction_attempts = 0;

                            let usage = usage.map(|usage| usage.with_invocation_id(invocation_id.clone()));
                            if let Some(ref usage) = usage {
                                terminal_usage = Some(usage.clone());
                                if let Some(reason) = usage.incomplete_reason.as_ref() {
                                    incomplete_reason_this_turn = Some(reason.clone());
                                }
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
                                        target: "permagent.context_packet",
                                        session_id = %session_config.id,
                                        provider_input_tokens = ?usage.usage.input_tokens,
                                        provider_output_tokens = ?usage.usage.output_tokens,
                                        packet_estimated_input_tokens = ?context_packet.estimated_total_tokens,
                                        "coding model usage joined to context packet"
                                    );
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
                            }

                            if let Some(response) = response {
                                stream_committed = true;
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
                                        produced_text_this_turn = true;
                                        last_assistant_text = text;
                                    }
                                    messages_to_add.push(response);
                                    continue;
                                }
                                requested_tool_this_turn = true;

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

                                    // Lane X-lat: per-tool-call name lookup for
                                    // the timing detail recorded as each result
                                    // arrives off `combined` below. Built once,
                                    // before dispatch, since `request_id` is
                                    // consumed by `add_tool_response_with_metadata`
                                    // when its result lands.
                                    let tool_name_by_request_id: HashMap<String, String> =
                                        remaining_requests
                                            .iter()
                                            .map(|request| {
                                                let name = request
                                                    .tool_call
                                                    .as_ref()
                                                    .map(|call| call.name.to_string())
                                                    .unwrap_or_else(|_| "unknown".to_string());
                                                (request.id.clone(), name)
                                            })
                                            .collect();
                                    tool_names_this_turn.extend(tool_name_by_request_id.values().cloned());

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

                                    let tool_phase_start = std::time::Instant::now();
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
                                                                if let Some(name) =
                                                                    tool_name_by_request_id.get(&request_id)
                                                                {
                                                                    turn_timing.record_tool_call(
                                                                        name.clone(),
                                                                        tool_phase_start,
                                                                    );
                                                                }
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
                                    turn_timing.record_tool_phase(tool_phase_start);

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

                                // The response's reasoning goes on every
                                // tool-call message (DeepSeek/Kimi require
                                // reasoning_content on each; Gemini wants its
                                // thinking echoed), ONCE: `tool_call_history`
                                // removes the streamed copy a provider replays
                                // here. Text that came with the calls goes on
                                // the first one instead of being dropped.
                                let requests: Vec<&ToolRequest> = frontend_requests
                                    .iter()
                                    .chain(remaining_requests.iter())
                                    .collect();
                                let ToolCallHistory { lead, calls } = {
                                    let mut pending: Vec<Message> =
                                        std::mem::take(&mut messages_to_add).into_iter().collect();
                                    let history = tool_call_history(&mut pending, &response, &requests);
                                    messages_to_add = Conversation::new_unvalidated(pending);
                                    history
                                };
                                if let Some(lead) = lead {
                                    messages_to_add.push(lead);
                                }

                                for (request, request_msg) in requests.iter().zip(calls) {
                                    if let Some(request_msg) = request_msg {
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
                        Err(ref provider_err @ ProviderError::ContextLengthExceeded(_)) => {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());

                            // Right after a failover to a LOCAL model, an
                            // overflow is the stand-in's small window, not a
                            // long history: the 2026-09-19 QA CLI run's system
                            // prompt and tools alone came to 62,364 tokens
                            // against the 32,768 Ollama serves qwen2.5:7b in
                            // (cli log 20260919_030958). The installed build
                            // let Ollama truncate that silently at n_ctx 4096;
                            // the Ollama preflight now refuses it instead. The
                            // switch refuses a model that small by its own
                            // count, so this arm is for when the preflight
                            // still says no. Compacting would have the 7B
                            // stand-in summarise the user's conversation and
                            // still end on "context limit still exceeded".
                            //
                            // Ending the turn is not enough on its own: the
                            // switch saved the stand-in as the session's model,
                            // so the user's resend went back to the same window,
                            // and `reply()`'s start-of-turn auto-compact judged
                            // the last cloud request's token count against the
                            // stand-in's window and summarised the history
                            // anyway. Switch the session back to the model it
                            // failed over from before saying so.
                            if let Some(previous) = failed_over_from.clone() {
                                let live = self.provider().await.ok().map(|p| {
                                    crate::cost_router::role_map::RoleModel {
                                        provider: p.get_name().to_string(),
                                        model: p.get_model_config().model_name,
                                    }
                                });
                                if let Some(stand_in) = live.filter(|t| {
                                    crate::cost_router::fallback::is_local_model(&t.provider, &t.model)
                                }) {
                                    error!("Error: {}", provider_err);
                                    self.record_context_length_failure(provider_err.to_string()).await;
                                    let from = previous.get_name().to_string();
                                    let restored = match self
                                        .update_provider(Arc::clone(&previous), &session_config.id)
                                        .await
                                    {
                                        Ok(()) => true,
                                        Err(e) => {
                                            warn!(
                                                session_id = %session_config.id,
                                                to_provider = %from,
                                                error = %e,
                                                "could not switch back from a local stand-in that was too small"
                                            );
                                            false
                                        }
                                    };
                                    tracing::warn!(
                                        target: "permagent::cost_router",
                                        session_id = %session_config.id,
                                        from_provider = %stand_in.provider,
                                        from_model = %stand_in.model,
                                        to_provider = %from,
                                        restored,
                                        reason = "local_fallback_overflow",
                                        "routing snapshot: switched back after the local stand-in's window refused the request"
                                    );
                                    let message = Message::assistant().with_system_notification(
                                        SystemNotificationType::InlineMessage,
                                        crate::cost_router::fallback::local_fallback_overflow_reply(
                                            &from,
                                            &stand_in,
                                            restored,
                                            failed_over_permanent_cause.as_deref(),
                                        ),
                                    );
                                    persist_turn_ending_message(&session_manager, &session_config.id, &message).await;
                                    yield AgentEvent::Message(message);
                                    state_guard.mark_error();
                                    break;
                                }
                            }

                            compaction_attempts += 1;

                            if compaction_attempts >= 2 {
                                error!("Context limit exceeded after compaction - prompt too large");
                                // R46a (harness loop, "A43 analysis" lever 3):
                                // the turn's own visible text below is
                                // deliberately scrubbed of the raw provider
                                // payload (it is shown to a human and read
                                // aloud); the RAW text — which for a local
                                // llama.cpp-shaped 400 carries the actual
                                // `n_prompt_tokens`/`n_ctx` numbers — is kept
                                // here instead, for a headless caller
                                // (`delegate`/`delegate_many`'s subagent) to
                                // read back via `take_context_length_failure`
                                // and turn into a classified reason.
                                self.record_context_length_failure(provider_err.to_string()).await;
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

                            let compact_provider = self.provider().await?;
                            let compact_accounting = self.accounted_fast_completion(
                                &session,
                                Arc::clone(&compact_provider),
                                true,
                            );
                            let compaction_started = std::time::Instant::now();
                            let compaction_result = compact_messages_accounted(
                                &session_config.id,
                                &conversation,
                                false,
                                compact_accounting.as_ref(),
                            )
                            .await;
                            turn_timing.record_context_mgmt(compaction_started);
                            match compaction_result {
                                Ok((compacted_conversation, usage)) => {
                                    session_manager.replace_conversation(&session_config.id, &compacted_conversation).await?;
                                    // Accounting occurred inside the reserved
                                    // fast-completion boundary before this
                                    // history replacement.
                                    let _ = usage;
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
                            // The RAW payload belongs in the log — request ids, JSON,
                            // support URLs. It must never reach the reply, which is
                            // typed on screen AND read aloud over TTS (2026-08-23).
                            error!("Error: {}", provider_err);

                            let failed_provider = match self.provider().await {
                                Ok(p) => p.get_name().to_string(),
                                Err(_) => "the model provider".to_string(),
                            };

                            let fallback = if permanent_failure_fallback_used {
                                None
                            } else {
                                let request_tokens = Self::failover_request_tokens(
                                    &system_prompt,
                                    conversation_with_moim.messages(),
                                    &tools,
                                )
                                .await;
                                self.switch_to_permanent_failure_fallback(
                                    &session_config.id,
                                    &failed_provider,
                                    provider_err,
                                    request_tokens,
                                    true,
                                ).await
                            };

                            let user_msg = crate::cost_router::fallback::permanent_failure_reply(
                                &failed_provider,
                                provider_err,
                                fallback.as_ref(),
                            );

                            if fallback.is_some() {
                                // Finish the turn on the new model instead of ending
                                // the session: the same shape as recovery compaction.
                                permanent_failure_fallback_used = true;
                                failed_over_from = Some(Arc::clone(&turn_provider));
                                failed_over_permanent_cause = Some(provider_err.user_facing_summary());
                                did_switch_provider_this_iteration = true;
                                let message = Message::assistant().with_system_notification(
                                    SystemNotificationType::InlineMessage,
                                    user_msg,
                                );
                                persist_turn_ending_message(&session_manager, &session_config.id, &message).await;
                                yield AgentEvent::Message(message);
                                break;
                            }

                            let notification_data = serde_json::json!({
                                "top_up_url": top_up_url,
                            });

                            let message = Message::assistant().with_system_notification_with_data(
                                SystemNotificationType::CreditsExhausted,
                                user_msg,
                                notification_data,
                            );
                            // Persist, so the explanation survives a reload rather
                            // than leaving the session ending on an unanswered turn.
                            persist_streamed_then_notice(
                                &session_manager,
                                &session_config.id,
                                &mut messages_to_add,
                                &mut conversation,
                                &message,
                            )
                            .await;
                            yield AgentEvent::Message(message);
                            state_guard.mark_error();
                            break;
                        }
                        // A transient failure INSIDE the stream (a dropped
                        // body, an in-band 429/5xx event) before anything
                        // reached the user: send the same call to the same
                        // model again after a bounded, jittered backoff that
                        // honours the provider's own Retry-After. Only when
                        // these retries are spent do the arms below fall back,
                        // and then for a bounded time. Session 20260924_3: an
                        // in-band TPM limit asking for 5 s moved a coding
                        // session to a review model for good.
                        Err(ref provider_err)
                            if crate::cost_router::fallback::same_model_retry_delay(
                                provider_err,
                                stream_committed,
                                same_model_retries,
                            )
                            .is_some() =>
                        {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            let delay = crate::cost_router::fallback::same_model_retry_delay(
                                provider_err,
                                stream_committed,
                                same_model_retries,
                            )
                            .unwrap_or_default();
                            same_model_retries += 1;
                            tracing::warn!(
                                target: "permagent::cost_router",
                                session_id = %session_config.id,
                                provider = %turn_provider.get_name(),
                                model = %turn_model_name,
                                cause = crate::cost_router::fallback::failover_cause(provider_err),
                                attempt = same_model_retries,
                                max_retries = crate::cost_router::fallback::SAME_MODEL_MAX_RETRIES,
                                delay_ms = delay.as_millis() as u64,
                                error = %provider_err,
                                "transient provider failure before any reply; retrying the same model"
                            );
                            sleep_before_same_model_retry(
                                turn_provider.get_name(),
                                provider_err,
                                delay,
                                same_model_retries,
                                &cancel_token,
                            )
                            .await;
                            did_retry_same_model_this_iteration = true;
                            resending_same_call = true;
                            break;
                        }
                        // `is_dropped_stream`, not `is_stream_decode`: a body a
                        // proxy or llama-server closed partway through a
                        // `data:` line reaches here as `RequestFailed("Failed
                        // to parse streaming chunk: EOF while parsing …")`,
                        // and the RequestFailed arm below told the user the
                        // provider "rejected this request as invalid" and to
                        // switch models, for a stream that simply dropped.
                        Err(ref provider_err)
                            if crate::cost_router::fallback::is_dropped_stream(provider_err) =>
                        {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            error!("Error: {}", provider_err);
                            // A dropped SSE body is transient. Before any assistant
                            // text was committed, silent-failover to the next model
                            // the same way a connect failure does. After commit, ask
                            // the user to resend — never "rejected as invalid".
                            if crate::cost_router::fallback::may_silent_precommit_failover(
                                stream_committed,
                                provider_err,
                            ) && !permanent_failure_fallback_used
                            {
                                let request_tokens = Self::failover_request_tokens(
                                    &system_prompt,
                                    conversation_with_moim.messages(),
                                    &tools,
                                )
                                .await;
                                if let Some(message) = self
                                    .fail_over_before_commit(
                                        &session_config.id,
                                        &turn_provider,
                                        provider_err,
                                        request_tokens,
                                        same_model_retries,
                                    )
                                    .await
                                {
                                    permanent_failure_fallback_used = true;
                                    failed_over_from = Some(Arc::clone(&turn_provider));
                                    failed_over_permanent_cause = None;
                                    did_switch_provider_this_iteration = true;
                                    // M2b: this arm was the last silent one. A
                                    // dropped SSE body switched the model with
                                    // nothing in the transcript to say so, which
                                    // is the same dishonesty the NetworkError
                                    // sibling below already stopped doing.
                                    yield AgentEvent::Message(message);
                                    break;
                                }
                            }
                            // Persisted, not just yielded. The chat client
                            // reloads the transcript from the daemon when the
                            // turn finishes, so an unpersisted final message is
                            // wiped milliseconds after it appears — which is
                            // how a dead turn ended up looking like silence.
                            let message = Message::assistant().with_text(
                                "The model stream dropped before the reply finished.\n\nPlease resend your message to try again."
                            );
                            persist_streamed_then_notice(
                                &session_manager,
                                &session_config.id,
                                &mut messages_to_add,
                                &mut conversation,
                                &message,
                            )
                            .await;
                            yield AgentEvent::Message(message);
                            state_guard.mark_error();
                            break;
                        }
                        Err(ref provider_err @ ProviderError::NetworkError(_)) => {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            error!("Error: {}", provider_err);
                            if crate::cost_router::fallback::may_silent_precommit_failover(
                                stream_committed,
                                provider_err,
                            ) && !permanent_failure_fallback_used
                            {
                                let request_tokens = Self::failover_request_tokens(
                                    &system_prompt,
                                    conversation_with_moim.messages(),
                                    &tools,
                                )
                                .await;
                                if let Some(message) = self
                                    .fail_over_before_commit(
                                        &session_config.id,
                                        &turn_provider,
                                        provider_err,
                                        request_tokens,
                                        same_model_retries,
                                    )
                                    .await
                                {
                                    permanent_failure_fallback_used = true;
                                    failed_over_from = Some(Arc::clone(&turn_provider));
                                    failed_over_permanent_cause = None;
                                    did_switch_provider_this_iteration = true;
                                    // M2: this switch used to be SILENT — the
                                    // one-line daemon warn was its entire trace,
                                    // so a session served by a different model
                                    // than its banner claimed had nothing in the
                                    // transcript to say so (20260831_10). Announce
                                    // it the way the CreditsExhausted sibling
                                    // already does. The turn still finishes on the
                                    // new model; only the silence is removed.
                                    yield AgentEvent::Message(message);
                                    break;
                                }
                            }
                            // Same reason as the stream-drop arm above: the
                            // client's end-of-turn reload replaces whatever is
                            // on screen with the daemon's persisted copy, so a
                            // terminal error has to be in that copy to survive.
                            let message = Message::assistant().with_text(
                                format!("{provider_err}\n\nPlease resend your message to try again.")
                            );
                            persist_streamed_then_notice(
                                &session_manager,
                                &session_config.id,
                                &mut messages_to_add,
                                &mut conversation,
                                &message,
                            )
                            .await;
                            yield AgentEvent::Message(message);
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
                            let failed_provider = match self.provider().await {
                                Ok(provider) => provider.get_name().to_string(),
                                Err(_) => String::new(),
                            };
                            let provider_label = if failed_provider.is_empty() {
                                "LLM provider".to_string()
                            } else {
                                format!("LLM provider ({failed_provider})")
                            };
                            // An auth rejection is permanent for THIS key: try the
                            // configured fallback exactly once before giving up.
                            // The BARE provider id goes to the resolver — it compares
                            // against configured provider ids, and a display label
                            // would never match, silently offering a fallback on the
                            // very provider whose key was just rejected.
                            let fallback = if permanent_failure_fallback_used {
                                None
                            } else {
                                let request_tokens = Self::failover_request_tokens(
                                    &system_prompt,
                                    conversation_with_moim.messages(),
                                    &tools,
                                )
                                .await;
                                self.switch_to_permanent_failure_fallback(
                                    &session_config.id,
                                    &failed_provider,
                                    provider_err,
                                    request_tokens,
                                    true,
                                ).await
                            };

                            if let Some(ref target) = fallback {
                                permanent_failure_fallback_used = true;
                                failed_over_from = Some(Arc::clone(&turn_provider));
                                failed_over_permanent_cause = Some(provider_err.user_facing_summary());
                                did_switch_provider_this_iteration = true;
                                let message = Message::assistant().with_system_notification(
                                    SystemNotificationType::InlineMessage,
                                    format!(
                                        "Authentication failed for your {provider_label}: the API key was rejected. \
                                         Switching to {} for the rest of this turn.",
                                        target.model
                                    ),
                                );
                                persist_turn_ending_message(&session_manager, &session_config.id, &message).await;
                                yield AgentEvent::Message(message);
                                break;
                            }

                            // No raw payload in the reply: an auth body can carry a
                            // key fragment, and this text is also read aloud.
                            let message = Message::assistant().with_text(
                                format!(
                                    "Authentication failed for your {provider_label}: the API key was rejected. \
                                     This is a credential problem with the model provider configured in Permagent — \
                                     not with any website, page, or service the agent was reading. \
                                     Check that the provider's API key is set, valid, and has the required permissions, then resend your message."
                                )
                            );
                            persist_streamed_then_notice(
                                &session_manager,
                                &session_config.id,
                                &mut messages_to_add,
                                &mut conversation,
                                &message,
                            )
                            .await;
                            yield AgentEvent::Message(message);
                            state_guard.mark_error();
                            break;
                        }
                        Err(ref provider_err @ ProviderError::RateLimitExceeded { .. }) => {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            error!("Error: {}", provider_err);
                            // By the time this arm runs the retry layer has already
                            // waited out its whole budget (see
                            // `providers::retry::RATE_LIMIT_MAX_RETRIES`), so there is
                            // nothing left to try automatically.
                            //
                            // What it must NOT do is answer as the assistant. Until
                            // 2026-08-25 a spent rate limit became "Ran into this
                            // error: …" in the assistant's own voice, which is wrong
                            // twice over: it reads as the model speaking, and it is
                            // then fed back into the model's context on the next turn
                            // as something the model said. A system notification says
                            // the same thing to the user and is dropped at format time.
                            let provider_id = match self.provider().await {
                                Ok(provider) => provider.get_name().to_string(),
                                Err(_) => String::new(),
                            };
                            // Before giving up, move a turn that has shown the
                            // user nothing yet. `may_silent_precommit_failover`
                            // has always accepted a spent rate limit, but this
                            // arm sits ahead of the generic one that called it,
                            // so a rate-limited cloud model ended the turn even
                            // with a free local model up on this machine.
                            if crate::cost_router::fallback::may_silent_precommit_failover(
                                stream_committed,
                                provider_err,
                            ) && !permanent_failure_fallback_used
                            {
                                let request_tokens = Self::failover_request_tokens(
                                    &system_prompt,
                                    conversation_with_moim.messages(),
                                    &tools,
                                )
                                .await;
                                if let Some(message) = self
                                    .fail_over_before_commit(
                                        &session_config.id,
                                        &turn_provider,
                                        provider_err,
                                        request_tokens,
                                        same_model_retries,
                                    )
                                    .await
                                {
                                    permanent_failure_fallback_used = true;
                                    failed_over_from = Some(Arc::clone(&turn_provider));
                                    failed_over_permanent_cause = None;
                                    did_switch_provider_this_iteration = true;
                                    yield AgentEvent::Message(message);
                                    break;
                                }
                            }
                            let message = Message::assistant().with_system_notification(
                                SystemNotificationType::InlineMessage,
                                format!(
                                    "{} — waited and retried, and it is still limited. \
                                     Your message was not sent; send it again when the limit clears.",
                                    crate::providers::wait_status::rate_limit_headline(&provider_id),
                                ),
                            );
                            persist_streamed_then_notice(
                                &session_manager,
                                &session_config.id,
                                &mut messages_to_add,
                                &mut conversation,
                                &message,
                            )
                            .await;
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
                            persist_streamed_then_notice(
                                &session_manager,
                                &session_config.id,
                                &mut messages_to_add,
                                &mut conversation,
                                &message,
                            )
                            .await;
                            yield AgentEvent::Message(message);
                            state_guard.mark_error();
                            break;
                        }
                        // Lane R19a (harness loop 2026-09-14, M19 DeepSeek
                        // forensics): a stall before the provider sent even
                        // one byte is not the same failure as a mid-stream
                        // stall or a repeating model (Y8a) — it is commonly
                        // provider-side queueing/prefill on a large context,
                        // and the M17-vs-M19 comparison showed an
                        // equivalent-sized request to a different provider,
                        // same binary, same moment, answering in seconds.
                        // Retry the identical request exactly once —
                        // `stream_committed` confirms nothing from THIS
                        // stream has reached the user yet, so there is no
                        // partial answer a second attempt could contradict.
                        //
                        // Lane R19c (G24 MED-1 correction): what actually
                        // keeps THIS arm from tripping Lane R18f's rule5
                        // guard is that it never ends the turn at all — it
                        // `continue`s the stream loop on a successful retry.
                        // Recording the stall as a `tracing::warn!`
                        // observability signal here, rather than an
                        // assistant message, is about not narrating an
                        // internal retry to the model/user as if it were the
                        // model's own text — it is not what saves this
                        // iteration from rule5; not ending the turn is.
                        Err(ref provider_err)
                            if should_retry_first_byte_stall(
                                provider_err,
                                stream_committed,
                                first_byte_stall_retried,
                            ) =>
                        {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            first_byte_stall_retried = true;
                            // Lane R19c (G24 MED-2): `crate::receipts::record`
                            // is a NO-OP for any (area, key) pair not already
                            // in `receipts::FIELDS` — a fixed-size const array
                            // `permagent-eval`'s regex parses by exact key
                            // set (see `safety_and_trust`'s own comment on
                            // why an unknown key there breaks the whole
                            // line). "provider"/"first_byte_stall_retry" is
                            // not one of the 14 existing pairs, and adding it
                            // changes that contract for every other
                            // consumer — out of scope for this lane to
                            // decide. `tracing::warn!` below is the
                            // observability signal instead; a scorecard-
                            // visible counter is a receipts-contract change,
                            // not a stream-budget one.
                            tracing::warn!(
                                target: "permagent::provider_stall",
                                session_id = %session_config.id,
                                provider = %turn_provider.get_name(),
                                model = %turn_model_name,
                                error = %provider_err,
                                "provider_stall: first-byte stall, retrying the identical request once (Lane R19a)"
                            );
                            match retry_first_byte_stall(
                                &session_manager,
                                &invocation,
                                first_byte_wait_started,
                                Self::stream_response_from_provider(
                                    turn_provider.clone(),
                                    &session_config.id,
                                    &system_prompt,
                                    conversation_with_moim.messages(),
                                    &tools,
                                    &toolshim_tools,
                                    invocation.reserved_output_tokens,
                                ),
                            )
                            .await
                            {
                                Ok(fresh) => {
                                    stream = fresh;
                                    continue;
                                }
                                Err(retry_error) => {
                                    tracing::warn!(
                                        target: "permagent::provider_stall",
                                        session_id = %session_config.id,
                                        provider = %turn_provider.get_name(),
                                        error = %retry_error,
                                        "provider_stall: the retry attempt itself could not start a new stream"
                                    );
                                    // Nothing reached the user and the stalled
                                    // provider cannot be retried, so this is the
                                    // same place as the give-up arm below: move
                                    // the turn to a model that is up rather than
                                    // ask for a resend. This branch used to end
                                    // the turn with no failover attempt at all.
                                    // The clause is the stall's own ("sent
                                    // nothing back in time"), not the
                                    // reservation error that stopped the retry.
                                    if crate::cost_router::fallback::may_silent_precommit_failover(
                                        stream_committed,
                                        provider_err,
                                    ) && !permanent_failure_fallback_used
                                    {
                                        let request_tokens = Self::failover_request_tokens(
                                            &system_prompt,
                                            conversation_with_moim.messages(),
                                            &tools,
                                        )
                                        .await;
                                        if let Some(message) = self
                                            .fail_over_before_commit(
                                                &session_config.id,
                                                &turn_provider,
                                                provider_err,
                                                request_tokens,
                                                same_model_retries,
                                            )
                                            .await
                                        {
                                            permanent_failure_fallback_used = true;
                                            failed_over_from = Some(Arc::clone(&turn_provider));
                                            failed_over_permanent_cause = None;
                                            did_switch_provider_this_iteration = true;
                                            yield AgentEvent::Message(message);
                                            break;
                                        }
                                    }
                                    let message = Message::assistant().with_system_notification(
                                        SystemNotificationType::InlineMessage,
                                        format!(
                                            "{} did not respond in time — this was retried once and still did not \
                                             respond. Your message was not sent; send it again once the provider \
                                             recovers.",
                                            turn_provider.get_name(),
                                        ),
                                    );
                                    persist_streamed_then_notice(
                                        &session_manager,
                                        &session_config.id,
                                        &mut messages_to_add,
                                        &mut conversation,
                                        &message,
                                    )
                                    .await;
                                    yield AgentEvent::Message(message);
                                    state_guard.mark_error();
                                    break;
                                }
                            }
                        }
                        // The retry above was already spent (or did not apply —
                        // content had already streamed) and this turn stalled
                        // before the first byte again. Give up the same way
                        // the rate-limit arm above does: a system
                        // notification in the assistant's own voice, never
                        // "Ran into this error… do not retry" advice text.
                        //
                        // Lane R19c (G24 MED-1 correction): this does NOT
                        // prevent Lane R18f's rule5 guard from firing.
                        // `Rule5HonestFailureGuard` (after_turn.rs) checks
                        // only for a tool call / `escalate` on the turn's
                        // final message (`final_message_has_tool_call` /
                        // `turn_called_escalate`) — Text vs
                        // SystemNotification content is irrelevant to it, and
                        // this arm calls neither a tool nor `escalate`. Rule5
                        // firing here is the CORRECT, intended outcome: the
                        // provider genuinely did not answer twice in a row,
                        // the coordinator has nothing left to do this turn,
                        // and an honest "this run could not make progress"
                        // is what the guard exists to produce — not a defect
                        // to be engineered around. What this lane actually
                        // fixes is upstream of rule5: the turn now reaches
                        // this give-up only after one real retry, with the
                        // reason recorded as a `provider_stall`
                        // observability signal, instead of every attempt
                        // burning the model's own turn on "do not retry the
                        // same request" free text it had no way to act on.
                        Err(ref provider_err) if provider_err.is_first_byte_provider_stall() => {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            tracing::warn!(
                                target: "permagent::provider_stall",
                                session_id = %session_config.id,
                                provider = %turn_provider.get_name(),
                                model = %turn_model_name,
                                error = %provider_err,
                                retried = first_byte_stall_retried,
                                "provider_stall: first-byte stall persisted; ending the turn without a retry left"
                            );
                            // The provider stayed silent through its one retry.
                            // Nothing reached the user, so move the turn to the
                            // next model that is actually up instead of asking
                            // for a resend (the split under load, or a cloud
                            // queue, while Ollama sits idle on this machine).
                            if crate::cost_router::fallback::may_silent_precommit_failover(
                                stream_committed,
                                provider_err,
                            ) && !permanent_failure_fallback_used
                            {
                                let request_tokens = Self::failover_request_tokens(
                                    &system_prompt,
                                    conversation_with_moim.messages(),
                                    &tools,
                                )
                                .await;
                                if let Some(message) = self
                                    .fail_over_before_commit(
                                        &session_config.id,
                                        &turn_provider,
                                        provider_err,
                                        request_tokens,
                                        same_model_retries,
                                    )
                                    .await
                                {
                                    permanent_failure_fallback_used = true;
                                    failed_over_from = Some(Arc::clone(&turn_provider));
                                    failed_over_permanent_cause = None;
                                    did_switch_provider_this_iteration = true;
                                    yield AgentEvent::Message(message);
                                    break;
                                }
                            }
                            let message = Message::assistant().with_system_notification(
                                SystemNotificationType::InlineMessage,
                                format!(
                                    "{} — {}. Your message was not sent; send it again once the provider recovers.",
                                    turn_provider.get_name(),
                                    provider_err.user_facing_summary(),
                                ),
                            );
                            persist_streamed_then_notice(
                                &session_manager,
                                &session_config.id,
                                &mut messages_to_add,
                                &mut conversation,
                                &message,
                            )
                            .await;
                            yield AgentEvent::Message(message);
                            state_guard.mark_error();
                            break;
                        }
                        Err(ref provider_err) => {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            error!("Error: {}", provider_err);
                            if crate::cost_router::fallback::may_silent_precommit_failover(
                                stream_committed,
                                provider_err,
                            ) && !permanent_failure_fallback_used
                            {
                                let request_tokens = Self::failover_request_tokens(
                                    &system_prompt,
                                    conversation_with_moim.messages(),
                                    &tools,
                                )
                                .await;
                                if let Some(message) = self
                                    .fail_over_before_commit(
                                        &session_config.id,
                                        &turn_provider,
                                        provider_err,
                                        request_tokens,
                                        same_model_retries,
                                    )
                                    .await
                                {
                                    permanent_failure_fallback_used = true;
                                    failed_over_from = Some(Arc::clone(&turn_provider));
                                    failed_over_permanent_cause = None;
                                    did_switch_provider_this_iteration = true;
                                    // The last arm that still switched in
                                    // silence. A 5xx from the split inside its
                                    // nightly window (or any cloud 503) moved
                                    // the session to the Ollama stand-in with
                                    // only a log line to say so, so the user
                                    // could not tell which model answered. Same
                                    // announcement as the NetworkError and
                                    // stream-drop siblings; the clause picks
                                    // "returned a server error" by variant.
                                    yield AgentEvent::Message(message);
                                    break;
                                }
                            }
                            // G13 L6: "please retry" is a lie for an error the
                            // retry layer has already refused to retry. A
                            // stream-budget breach is OURS — we stopped the
                            // reply — and resending the identical request walks
                            // straight back into the same runaway.
                            let advice = if provider_err.is_stream_budget_exceeded() {
                                "We stopped this reply ourselves: it ran past a streaming safety budget. \
                                 Do not retry the same request — it would hit the same bound. \
                                 Continue from the work that is already done, or split what is left into \
                                 smaller steps and take the first one."
                            } else if provider_err.is_permanent() {
                                "This failure is permanent — retrying the same request will not help. \
                                 Proceed with what you have, or take a different approach."
                            } else {
                                "Please retry if you think this is a transient or recoverable error."
                            };
                            let message = Message::assistant().with_text(
                                format!("Ran into this error: {provider_err}.\n\n{advice}")
                            );
                            persist_streamed_then_notice(
                                &session_manager,
                                &session_config.id,
                                &mut messages_to_add,
                                &mut conversation,
                                &message,
                            )
                            .await;
                            yield AgentEvent::Message(message);
                            state_guard.mark_error();
                            break;
                        }
                    }
                }
                turn_timing.end_model();
                let call_outcome = stream_outcome.finish(is_token_cancelled(&cancel_token));
                if invocation.cost_tier.is_chargeable() {
                    let has_authoritative_usage = terminal_usage.as_ref().is_some_and(|usage| {
                        let u = usage.usage;
                        u.input_tokens.is_some()
                            || u.output_tokens.is_some()
                            || u.total_tokens.is_some()
                            || u.cache_read_input_tokens.is_some()
                            || u.cache_write_input_tokens.is_some()
                    });
                    if let Some(usage) = terminal_usage.as_ref().filter(|_| has_authoritative_usage)
                    {
                        if let Err(error) = self.update_session_metrics_for_invocation(
                            &session_config.id,
                            session_config.schedule_id.clone(),
                            usage,
                            &invocation,
                            recognition_retrieval_id.as_deref(),
                            call_outcome,
                        )
                        .await
                        {
                            if let Some(reservation_id) = invocation.reservation_id.as_deref() {
                                // Settlement failure is not evidence that the
                                // remote provider did not charge us. Preserve
                                // the hold as unknown before surfacing the
                                // durable accounting error.
                                session_manager
                                    .mark_provider_invocation_unknown(reservation_id)
                                    .await?;
                            }
                            let message = Message::assistant().with_text(format!(
                                "Provider usage was received, but durable accounting failed: {error}."
                            ));
                            persist_turn_ending_message(
                                &session_manager,
                                &session_config.id,
                                &message,
                            )
                            .await;
                            yield AgentEvent::Message(message);
                            state_guard.mark_error();
                            break;
                        }
                    } else if let Some(reservation_id) = invocation.reservation_id.as_deref() {
                        crate::cost_router::authorized_attempts::finalize_missing_usage(
                            &session_manager, reservation_id, rejected_without_usage,
                        ).await?;
                    }
                } else if let Some(usage) = terminal_usage.as_ref() {
                    self.update_session_metrics_for_invocation(
                        &session_config.id,
                        session_config.schedule_id.clone(),
                        usage,
                        &invocation,
                        recognition_retrieval_id.as_deref(),
                        call_outcome,
                    )
                    .await?;
                }
                // v60 (node A4): both flags are set only from provider-error
                // arms, and each makes the loop re-run this turn. That re-run
                // is a re-dispatch after this invocation failed.
                if !did_retry_same_model_this_iteration {
                    same_model_retries = 0;
                }
                retry_of_failed_call = (did_switch_provider_this_iteration
                    || did_recovery_compact_this_iteration
                    || did_retry_same_model_this_iteration)
                    .then(|| invocation.invocation_id.clone());
                // A mid-turn failover re-runs this turn on a different model,
                // so the prompt must be rebuilt for it. `prepare_tools_and_prompt`
                // is where the live model identity line and the model family's
                // overlay are written, and the re-run otherwise reused the
                // prompt built for the model that just failed: the Ollama
                // stand-in was told it was claude-haiku-4-5, the 2026-08-31
                // wrong-identity defect again, one layer down.
                let refresh_prompt = tools_updated || did_switch_provider_this_iteration;
                if refresh_prompt {
                    (tools, toolshim_tools, system_prompt) =
                        self.prepare_tools_and_prompt(&session_config.id, &session.working_dir).await?;
                }

                {
                    let has_new_hints = self
                        .prompt_manager
                        .lock()
                        .await
                        .load_subdirectory_hints(&working_dir);
                    if has_new_hints && !refresh_prompt {
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

                // Y-poll-suspend: evaluated once per completed turn, using
                // ONLY this turn's own tool calls and a fresh read of this
                // coordinator's dispatched-goal state — see
                // `DispatchPollGuard` and `suspend_for_dispatched_goals`
                // above for the full contract and the measured cost this
                // exists to cut. A turn that called no tools at all
                // (`no_tools_called`) is the S5 monologue guard's problem,
                // not this one's — there is nothing dispatched-worker-shaped
                // to have looked at.
                if !no_tools_called {
                    let drain_pool = self.config.session_manager.pool_clone().await.ok();
                    let snapshot = match drain_pool.as_ref() {
                        Some(pool) => {
                            match crate::agents::platform_extensions::orchestrator::coordinator_worker_state_snapshot(
                                pool,
                                &session_config.id,
                            )
                            .await
                            {
                                Ok(snapshot) if !snapshot.is_empty() => Some(snapshot),
                                Ok(_) => None,
                                Err(error) => {
                                    tracing::debug!(
                                        target: "permagentd::brain",
                                        session_id = %session_config.id,
                                        %error,
                                        "Y-poll-suspend: could not read dispatched-goal state for \
                                         this coordinator; treating this turn as having nothing \
                                         dispatched rather than risking a false suspend"
                                    );
                                    None
                                }
                            }
                        }
                        None => None,
                    };
                    let should_suspend =
                        dispatch_poll_guard.observe_turn(&tool_names_this_turn, snapshot);
                    if should_suspend {
                        if let Some(pool) = drain_pool.as_ref() {
                            let drain_cap_secs: u64 = Config::global()
                                .get_param(MID_TURN_DISPATCH_SUSPEND_SECS_KEY)
                                .unwrap_or(DEFAULT_MID_TURN_DISPATCH_SUSPEND_CAP_SECS);
                            let notice = suspend_for_dispatched_goals(
                                pool,
                                &session_config.id,
                                std::time::Duration::from_secs(drain_cap_secs),
                                &cancel_token,
                            )
                            .await;
                            match notice {
                                Some(notice) => {
                                    dispatch_poll_guard.record_resume();
                                    let message = Message::user().with_text(notice);
                                    messages_to_add.push(message.clone());
                                    yield AgentEvent::Message(message);
                                }
                                None => {
                                    // Cancelled mid-wait: append nothing extra
                                    // and let this loop's ordinary
                                    // cancellation check, at the very top of
                                    // the next iteration, end the reply the
                                    // same way it always does — this guard
                                    // has no special-cased cleanup of its own.
                                }
                            }
                        }
                    }
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
                // The empty-turn bound is for turns IN A ROW: any turn that said
                // something or asked for a tool starts it again.
                if produced_text_this_turn || requested_tool_this_turn {
                    empty_turn_continuations = 0;
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
                        None if did_recovery_compact_this_iteration
                            || did_switch_provider_this_iteration
                            || did_retry_same_model_this_iteration => {
                            // Continue from the last user message: either the history was
                            // just compacted, or the model was just swapped after a
                            // permanent provider rejection. Both changed the inputs, so
                            // the turn is worth re-running rather than ending.
                        }
                        None => {
                            let empty_turn = decide_empty_turn(
                                produced_text_this_turn,
                                requested_tool_this_turn,
                                incomplete_reason_this_turn.as_deref(),
                                empty_turn_continuations,
                            );
                            match empty_turn {
                                EmptyTurnDecision::Continue { prompt } => {
                                    // Not the end of the run: the model produced
                                    // nothing (a reasoning-only or incomplete
                                    // response). Re-prompt the SAME session.
                                    empty_turn_continuations += 1;
                                    warn!(
                                        session_id = %session_config.id,
                                        continuation = empty_turn_continuations,
                                        incomplete_reason = ?incomplete_reason_this_turn,
                                        "assistant turn produced no text and no tool call; \
                                         continuing the run instead of ending it"
                                    );
                                    let message = Message::user().with_text(prompt);
                                    messages_to_add.push(message.clone());
                                    yield AgentEvent::Message(message);
                                }
                                decision => {
                                    if let EmptyTurnDecision::GiveUp { reason } = &decision {
                                        warn!(
                                            session_id = %session_config.id,
                                            incomplete_reason = ?incomplete_reason_this_turn,
                                            "{reason}"
                                        );
                                    }
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
                    }
                }

                // R28a (HARNESS_LOOP_LOG.md "A27 analysis" misses 1/2): tell a
                // dispatched goal worker plainly, once, when only a handful of
                // turns remain before the hard `max_turns` cap below discards
                // whatever it has not yet committed. Independent of whether
                // THIS turn called a tool — the failure mode is a worker that
                // keeps calling tools (manual shell re-tests) right up to the
                // cap, never a monologuing one, so this cannot live inside the
                // `no_tools_called` branch above. `has_explicit_cap` (`session_
                // config.max_turns.is_some()`) is what keeps this off ordinary
                // interactive chats — see `turn_budget_nudge_text`'s own doc
                // comment. R62f: and `turn_budget_nudge_applies` keeps it off a
                // capped top-level coordinator.
                if !turn_budget_nudged {
                    if let Some(nudge_text) = crate::after_turn::turn_budget_nudge_text(
                        turns_taken,
                        max_turns,
                        turn_budget_nudge_eligible,
                    ) {
                        turn_budget_nudged = true;
                        let nudge = Message::user().with_text(nudge_text);
                        messages_to_add.push(nudge.clone());
                        yield AgentEvent::Message(nudge);
                    }
                }

                // ── Make the turn WELL-FORMED before anything is added to it ──
                //
                // One provider turn can arrive as SEVERAL assistant messages:
                // any provider that emits reasoning before its tool call (this
                // was MiniMax-M2.7) streams thinking/text in one item and the
                // tool request in the next, and the loop above pushed each item
                // as its own assistant row. The result is consecutive
                // same-role assistant messages — a sequence no provider
                // accepts, repaired downstream on every single turn.
                //
                // That repair is what the "merged consecutive same-role
                // messages" warnings were: session 20260825_3 logged 22 of them
                // in one conversation, one per tool-calling turn. MOIM was
                // fixing this correctly and warning, correctly, that something
                // upstream was producing it. This is that something.
                //
                // Fixed at the source with the SAME lossless merge MOIM relies
                // on: consecutive same-role content is concatenated in order,
                // nothing is dropped, and the reasoning ends up attached to the
                // tool call it belongs to — which is also what Gemini/Kimi
                // require to accept the next request.
                //
                // Runs HERE, before the tool-pair summaries are appended below:
                // each summary stands in for one tool pair and has to remain its
                // own message, so folding them into each other would collapse a
                // whole batch into one.
                let (coalesced, merges) =
                    coalesce_turn_messages(messages_to_add.messages().clone());
                if merges > 0 {
                    debug!(
                        session_id = %session_config.id,
                        merges,
                        "coalesced a streamed turn into alternating messages before persisting"
                    );
                }
                messages_to_add = Conversation::new_unvalidated(coalesced);

                if is_token_cancelled(&cancel_token) {
                    tool_pair_summarization_task.abort();
                }

                let summarization_started = std::time::Instant::now();
                let summarization_result = tool_pair_summarization_task.await;
                turn_timing.record_context_mgmt(summarization_started);
                if let Ok(summaries) = summarization_result {
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

                // ── After-turn hooks (deepagents `after_agent`, which can jump
                // back to the model). This is the ONE place the reply loop
                // decides a turn is over, so it is the only place a guard has
                // to attach to. `Continue` re-enters the model loop with an
                // injected instruction instead of finishing — the interactive
                // twin of `HoldOutcome::Hold { inject_plan }`, which until now
                // only ever fired on a goal card's InProgress → Review edge.
                let after_turn_started = std::time::Instant::now();
                if exit_chat && !self.after_turn_manager.is_empty() {
                    let action = self
                        .after_turn_manager
                        .after_turn(&crate::after_turn::AfterTurnContext {
                            session_id: &session_config.id,
                            messages: conversation.messages(),
                            prior_holds: after_turn_holds,
                            // `tools` is this turn's resolved tool list (same
                            // one sent to the provider) — CASE B: a recipe
                            // that never loaded `summon` has no `delegate`
                            // tool at all, and `ReviewerMandate` must know
                            // that instead of asking for a tool that can't be
                            // called.
                            delegate_tool_available: crate::after_turn::delegate_tool_loaded(
                                &tools,
                            ),
                        })
                        .await;
                    match action {
                        crate::after_turn::AfterTurnAction::Allow => {}
                        crate::after_turn::AfterTurnAction::Continue { inject } => {
                            after_turn_holds = after_turn_holds.saturating_add(1);
                            exit_chat = false;
                            let message = Message::user().with_text(inject);
                            session_manager.add_message(&session_config.id, &message).await?;
                            conversation.push(message.clone());
                            yield AgentEvent::Message(message);
                        }
                        crate::after_turn::AfterTurnAction::Park { reason } => {
                            // The turn still ends — but never silently. The
                            // user is told what was refused and why.
                            let message = Message::assistant().with_text(reason);
                            persist_turn_ending_message(&session_manager, &session_config.id, &message).await;
                            yield AgentEvent::Message(message);
                        }
                    }
                }
                turn_timing.record_after_turn(after_turn_started);

                // Lane X-lat: attach this turn's wall-clock breakdown to its
                // already-settled `cost_ledger` row. Best-effort — a call_id
                // with no settled row (an unrecoverable pre-dispatch failure
                // broke out of the loop earlier and never reaches this point;
                // a recoverable error that produced no authoritative usage
                // settles no row either) is a harmless no-op, and a write
                // failure here must never fail the turn — this is
                // observability, not money accounting.
                let turn_timing_record = turn_timing.finish();
                if let Err(error) = session_manager
                    .update_cost_ledger_timing(&invocation_id, &turn_timing_record)
                    .await
                {
                    debug!(
                        session_id = %session_config.id,
                        %error,
                        "failed to persist per-turn harness timing (best-effort observability, not fatal)"
                    );
                }

                // Lane R17a (M17 readiness audit §6.1, gaps 1 & 4): refresh
                // the receipts sidecar's `harness_overhead` line at this turn
                // boundary too, not only at the session's clean exit — a
                // harness SIGKILLed for exceeding the eval's wall-clock
                // ceiling still leaves a recent-enough reading on disk for
                // `permagent-eval`'s timeout fallback
                // (`PERMAGENT_RECEIPTS_SIDECAR`) to read. `is_active` gates
                // this before any rollup query, so an ordinary run without a
                // sidecar configured pays no extra cost.
                if crate::receipts::is_active() {
                    if let Ok(rollup) =
                        session_manager.session_timing_rollup(&session_config.id).await
                    {
                        crate::receipts::set_harness_overhead_line(
                            crate::session::format_harness_overhead_line(&rollup),
                        );
                    }
                }

                if exit_chat {
                    break;
                }

                tokio::task::yield_now().await;
            }

            if !last_assistant_text.is_empty() {
                tracing::info!(target: "permagent::agents::agent", trace_output = last_assistant_text.as_str());
            }
            let outcome = runtime_outcome(
                state_guard.iteration_errored(),
                is_token_cancelled(&cancel_token),
            );
            yield AgentEvent::RuntimeOutcome(outcome);
        }.instrument(reply_stream_span));
        Ok(inner)
    }

    pub async fn extend_system_prompt(&self, key: String, instruction: String) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.add_system_prompt_extra(key, instruction);
    }

    /// Install a prompt extra and preserve its typed source attribution for
    /// the request-boundary ContextPacket. The model still receives the exact
    /// instruction text; attribution never relies on parsing that text later.
    pub async fn extend_system_prompt_with_context(
        &self,
        key: String,
        instruction: String,
        contribution: crate::context_packet::ContextContribution,
    ) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.add_system_prompt_extra_with_context(key, instruction, contribution);
    }

    pub async fn extend_system_prompt_with_contexts(
        &self,
        key: String,
        instruction: String,
        contributions: Vec<crate::context_packet::ContextContribution>,
    ) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.add_system_prompt_extra_with_contexts(key, instruction, contributions);
    }

    pub async fn set_persona(&self, persona: crate::config::agent_identity::SharedPersona) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.set_persona(persona);
    }

    /// The name this agent would be told it has, right now — worker override,
    /// else the live shared persona, else the last-known-good value.
    ///
    /// `prompt_manager` is `pub(super)`, so without this a caller outside
    /// `permagent::agents` (the CLI banner, and any test of it) could not read
    /// the installed identity back at all.
    pub async fn persona_display_name(&self) -> String {
        self.prompt_manager.lock().await.display_name()
    }

    /// The persona's own opening line, for a client-side greeting. Empty when
    /// the persona sets none or a worker override is installed.
    pub async fn persona_opening_greeting(&self) -> String {
        self.prompt_manager.lock().await.opening_greeting()
    }

    pub async fn set_persona_block_override(&self, block: String, display_name: String) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.set_persona_block_override(block, display_name);
    }

    pub async fn set_worker_key(&self, key: Option<String>) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.set_worker_key(key);
    }

    pub async fn update_provider(
        &self,
        provider: Arc<dyn Provider>,
        session_id: &str,
    ) -> Result<()> {
        let provider_name = provider.get_name().to_string();
        let model_config = provider.get_model_config();

        // An explicit switch (the user, the chat route, a permanent-failure
        // fallback, the overflow arm's switch back) supersedes any time-boxed
        // transient fallback: there is nothing left to go back from.
        *self.transient_failover.lock().await = None;
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

    /// Swap the live provider after the current one rejected the request
    /// permanently (billing / auth) or failed before anything was committed
    /// (see `fail_over_before_commit`), so the turn can finish on another
    /// model. `cause` is the error behind the switch, logged as the reason;
    /// `request_tokens` is the size of the request that failed, so a local
    /// model whose window cannot hold it is never chosen.
    ///
    /// Returns the target that is now live, or `None` when nothing is
    /// configured to fall back to — the caller must then say so plainly rather
    /// than retrying into the same wall. Errors from creating the fallback are
    /// swallowed into `None`: a broken fallback is the same user-visible
    /// situation as no fallback, and must never mask the original rejection.
    ///
    /// Regression context: session 20260823_4 (2026-08-23) burned every turn on
    /// an Anthropic "credit balance too low" 400 until the user noticed and
    /// switched to DeepSeek by hand.
    ///
    /// `sticky`: a permanent failure (billing / auth) persists the switch to
    /// the session record, because the failed model rejects every request
    /// until the operator fixes it. A transient one swaps the live provider in
    /// memory only and leaves the session record on the operator's model
    /// (`fail_over_before_commit` time-boxes it), so workers, an evicted
    /// agent and the next session start all still see the chosen model.
    async fn switch_to_permanent_failure_fallback(
        &self,
        session_id: &str,
        failed_provider: &str,
        cause: &ProviderError,
        request_tokens: Option<usize>,
        sticky: bool,
    ) -> Option<crate::cost_router::role_map::RoleModel> {
        #[cfg(test)]
        let injected = {
            self.test_failover_request_tokens
                .lock()
                .await
                .push(request_tokens);
            self.test_failover.lock().await.take()
        };
        #[cfg(not(test))]
        let injected: Option<(crate::cost_router::role_map::RoleModel, Arc<dyn Provider>)> = None;
        let (target, replaced, provider) = match injected {
            Some((target, provider)) => (target, String::new(), provider),
            None => {
                let role = self.failover_session_role(session_id).await;
                self.build_failover_provider(failed_provider, request_tokens, role)
                    .await?
            }
        };
        // For an Ollama target this is the `num_ctx` the provider will size
        // the request to. QA on 2026-09-19 could not confirm the stand-in's
        // window because no log carried it.
        let context_limit = provider.get_model_config().context_limit;

        if sticky {
            if let Err(e) = self.update_provider(provider, session_id).await {
                warn!(
                    fallback_provider = %target.provider,
                    fallback_model = %target.model,
                    error = %e,
                    "Could not activate the configured fallback provider"
                );
                return None;
            }
        } else {
            *self.provider.lock().await = Some(provider);
        }

        // Routing snapshot note: the ONE line that explains, after the fact, why
        // this session's spend moved to a different model mid-turn. `reason`
        // is the real cause: it used to say `permanent_provider_rejection` for
        // network, 5xx, rate-limit and stall failovers too, so the log gave
        // QA a false cause for every transient switch.
        tracing::warn!(
            target: "permagent::cost_router",
            session_id = %session_id,
            from_provider = %failed_provider,
            to_provider = %target.provider,
            to_model = %target.model,
            replaced_unavailable = %replaced,
            context_limit = ?context_limit,
            request_tokens = ?request_tokens,
            reason = crate::cost_router::fallback::failover_cause(cause),
            class = ?crate::cost_router::fallback::classify_failure(cause),
            sticky,
            "routing snapshot: switched model mid-turn"
        );

        Some(target)
    }

    /// Choose the fallback for `failed_provider` over live local availability
    /// and build its provider. Returns the target, the configured candidate it
    /// stands in for (empty when none), and the provider, or `None` when
    /// nothing configured can serve a request of `request_tokens`.
    async fn build_failover_provider(
        &self,
        failed_provider: &str,
        request_tokens: Option<usize>,
        role: crate::cost_router::fallback::SessionRole,
    ) -> Option<(
        crate::cost_router::role_map::RoleModel,
        String,
        Arc<dyn Provider>,
    )> {
        // Availability-aware: probe the local endpoints the configured list
        // names and never switch to one that is down. 2026-09-18: every
        // permanent failure switched to the nightly-only `qwen38_split`, whose
        // port is closed outside 01:50–06:10, and the turn then died on
        // "Could not connect to 127.0.0.1:8081" while Ollama was up with four
        // models pulled. A down local candidate is now replaced by the best
        // installed Ollama model (see `cost_router::fallback::choose_fallback`).
        //
        // Size-aware: a local model whose window cannot hold this request is
        // not chosen. 2026-09-19: the QA CLI run's 62,364-token request went
        // to qwen2.5:7b's 32,768 window (cli log 20260919_030958); the
        // installed build let Ollama truncate it at n_ctx 4096, and the Ollama
        // preflight now refuses a request that size, so choosing by
        // availability alone would move the turn to a model certain to refuse.
        let choice = crate::cost_router::fallback::failover_for_request(
            failed_provider,
            request_tokens,
            role,
        )
        .await?;
        let replaced = choice
            .replaced
            .as_ref()
            .map(|rm| format!("{}/{}", rm.provider, rm.model))
            .unwrap_or_default();
        let target = choice.target;

        let model_config = match crate::model::ModelConfig::new(&target.model) {
            Ok(mc) => {
                let mc = mc.with_canonical_limits(&target.provider);
                // An Ollama model has no canonical row, so it would otherwise
                // reach the provider with no `num_ctx` and be served through
                // Ollama's small default window — the agent's system prompt
                // alone overflows it. The knowledge-base row carries the
                // window the model is actually served with.
                if mc.context_limit.is_none() {
                    let limit = crate::cost_router::fallback::local_context_limit(
                        &target.provider,
                        &target.model,
                    );
                    mc.with_context_limit(limit)
                } else {
                    mc
                }
            }
            Err(e) => {
                warn!(
                    fallback_provider = %target.provider,
                    fallback_model = %target.model,
                    error = %e,
                    "Configured fallback model is invalid; continuing without a fallback"
                );
                return None;
            }
        };

        let extensions = self.get_extension_configs().await;
        let provider = match crate::providers::create(&target.provider, model_config, extensions)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    fallback_provider = %target.provider,
                    fallback_model = %target.model,
                    error = %e,
                    "Could not create the configured fallback provider; continuing without a fallback"
                );
                return None;
            }
        };
        Some((target, replaced, provider))
    }

    /// The size of the request that just failed, counted the way the
    /// reservation counts it, so the switch can refuse a local model whose
    /// window cannot hold it. `None` when it cannot be counted (images or
    /// audio in context); the switch then behaves as it did before sizing.
    async fn failover_request_tokens(
        system_prompt: &SystemPromptParts,
        messages: &[Message],
        tools: &[Tool],
    ) -> Option<usize> {
        crate::agents::reply_parts::counted_prompt_tokens(
            &[
                system_prompt.stable_prefix(),
                system_prompt.volatile_suffix(),
            ],
            messages,
            tools,
        )
        .await
    }

    /// Which seat this session fills, so its fallback is chosen for that job
    /// ([`crate::cost_router::fallback::failover_candidates`]). The coding
    /// harness's top-level coordinator is the one seat with a stricter gate;
    /// the same predicate the coding-DAG guards use decides it. A session the
    /// store cannot read is treated as general, which is the old behaviour.
    async fn failover_session_role(
        &self,
        session_id: &str,
    ) -> crate::cost_router::fallback::SessionRole {
        use crate::cost_router::fallback::SessionRole;
        match self
            .config
            .session_manager
            .get_session(session_id, false)
            .await
        {
            Ok(session)
                if crate::tool_inspection::is_top_level_coding_harness(
                    session.recipe.as_ref().map(|r| r.title.as_str()),
                    session.parent_session_id.as_deref(),
                ) =>
            {
                SessionRole::HarnessCoordinator
            }
            _ => SessionRole::General,
        }
    }

    /// End a time-boxed transient fallback: put the original provider back
    /// once its time is up (or at once, with `force`, at the start of a new
    /// reply), log it, and persist the one sentence that says so. `None` when
    /// no transient fallback is in force, it is not due, or something else
    /// has switched the provider since (then that switch stands).
    async fn end_transient_failover(&self, session_id: &str, force: bool) -> Option<Message> {
        let record = {
            let mut slot = self.transient_failover.lock().await;
            let due = slot
                .as_ref()
                .is_some_and(|t| force || std::time::Instant::now() >= t.until);
            if !due {
                return None;
            }
            slot.take()?
        };
        {
            let mut live = self.provider.lock().await;
            let still_ours = live
                .as_ref()
                .is_some_and(|p| Arc::ptr_eq(p, &record.fallback));
            if !still_ours {
                return None;
            }
            *live = Some(Arc::clone(&record.original));
        }
        let original = crate::cost_router::role_map::RoleModel {
            provider: record.original.get_name().to_string(),
            model: record.original.get_model_config().model_name,
        };
        tracing::info!(
            target: "permagent::cost_router",
            session_id = %session_id,
            to_provider = %original.provider,
            to_model = %original.model,
            from_provider = %record.fallback_target.provider,
            from_model = %record.fallback_target.model,
            forced_by_new_reply = force,
            "routing snapshot: transient fallback ended; back on the chosen model"
        );
        let message = Message::assistant().with_system_notification(
            SystemNotificationType::InlineMessage,
            crate::cost_router::fallback::transient_failover_restored_reply(
                &original,
                &record.fallback_target,
            ),
        );
        persist_turn_ending_message(&self.config.session_manager, session_id, &message).await;
        Some(message)
    }

    /// One pre-commit failover, shared by every transient arm of the reply
    /// loop (dropped stream, refused connection, 5xx, spent rate limit, first-
    /// byte stall): switch the session to the next model that is up and can
    /// hold the request, log why, and persist the one sentence that says so.
    /// Returns that sentence for the arm to yield, or `None` when nothing
    /// could take the turn and the arm should end it as before.
    ///
    /// Persisted, not just yielded: the desktop reloads the transcript when a
    /// turn ends, and an unpersisted notice vanishes with the reload.
    ///
    /// Every caller is a TRANSIENT arm (`may_silent_precommit_failover`
    /// refuses permanent errors), so the switch is time-boxed, not sticky:
    /// it is in memory only, and `end_transient_failover` puts `failed` back
    /// after `transient_failover_cooldown` or at the next reply. `retries` is
    /// how many same-model retries this call already spent, for the notice.
    async fn fail_over_before_commit(
        &self,
        session_id: &str,
        failed: &Arc<dyn Provider>,
        err: &ProviderError,
        request_tokens: Option<usize>,
        retries: usize,
    ) -> Option<Message> {
        let failed_provider = failed.get_name().to_string();
        let target = self
            .switch_to_permanent_failure_fallback(
                session_id,
                &failed_provider,
                err,
                request_tokens,
                false,
            )
            .await?;
        let cooldown = crate::cost_router::fallback::transient_failover_cooldown(err);
        if let Some(fallback) = self.provider.lock().await.as_ref().map(Arc::clone) {
            *self.transient_failover.lock().await = Some(TransientFailover {
                original: Arc::clone(failed),
                fallback,
                fallback_target: target.clone(),
                until: std::time::Instant::now() + cooldown,
            });
        }
        tracing::info!(
            target: "permagent::cost_router",
            session_id = %session_id,
            from_provider = %failed_provider,
            to_provider = %target.provider,
            to_model = %target.model,
            cause = crate::cost_router::fallback::failover_cause(err),
            same_model_retries = retries,
            cooldown_secs = cooldown.as_secs(),
            "pre-commit failover (transient, time-boxed)"
        );
        let message = Message::assistant().with_system_notification(
            SystemNotificationType::InlineMessage,
            crate::cost_router::fallback::transient_failover_reply(
                &failed_provider,
                err,
                &target,
                retries,
                cooldown,
            ),
        );
        persist_turn_ending_message(&self.config.session_manager, session_id, &message).await;
        Some(message)
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
        let (result, _usage) =
            crate::agents::reply_parts::AccountedFastCompletion::complete_accounted(
                Arc::clone(&self.config.session_manager),
                session,
                Arc::clone(&provider),
                &system_prompt,
                messages.messages(),
                &tools,
                false,
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

    #[derive(Clone, Copy)]
    enum OutcomeFirstReply {
        Failure,
        Text,
        Read,
    }

    struct OutcomeSequenceProvider {
        calls: std::sync::atomic::AtomicUsize,
        first_reply: OutcomeFirstReply,
        read_path: String,
        cancel_on_failure: Option<CancellationToken>,
    }

    #[async_trait::async_trait]
    impl Provider for OutcomeSequenceProvider {
        fn get_name(&self) -> &str {
            "outcome-sequence-fixture"
        }
        fn get_model_config(&self) -> crate::model::ModelConfig {
            crate::model::ModelConfig::new_or_fail("gpt-5.4-mini")
        }
        fn cost_tier(&self) -> crate::session::CostTier {
            crate::session::CostTier::LocalFree
        }
        async fn stream(
            &self,
            _model_config: &crate::model::ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> std::result::Result<crate::providers::base::MessageStream, ProviderError> {
            let first = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
            if first && matches!(self.first_reply, OutcomeFirstReply::Failure) {
                if let Some(cancel) = &self.cancel_on_failure {
                    cancel.cancel();
                }
                return Err(ProviderError::NotSent(
                    "fixture admission refusal".to_string(),
                ));
            }
            let message = if first && matches!(self.first_reply, OutcomeFirstReply::Read) {
                Message::assistant().with_tool_request(
                    "fixture-read",
                    Ok(CallToolRequestParams::new("developer__read")
                        .with_arguments(rmcp::object!({"path": self.read_path.clone()}))),
                )
            } else {
                Message::assistant().with_text("Recovered successfully.")
            };
            Ok(crate::providers::base::stream_from_single_message(
                message,
                ProviderUsage::new(
                    self.get_name().to_string(),
                    crate::providers::base::Usage::default(),
                ),
            ))
        }
    }

    #[tokio::test]
    async fn reply_outcome_preserves_failure_at_cap_and_allows_real_recovery() -> anyhow::Result<()>
    {
        for (first_reply, max_turns, cancel_after_failure, expected, expected_calls) in [
            (
                OutcomeFirstReply::Failure,
                1,
                false,
                AgentRuntimeOutcome::Failed,
                1,
            ),
            (
                OutcomeFirstReply::Failure,
                2,
                false,
                AgentRuntimeOutcome::Succeeded,
                2,
            ),
            (
                OutcomeFirstReply::Failure,
                2,
                true,
                AgentRuntimeOutcome::Cancelled,
                1,
            ),
            (
                OutcomeFirstReply::Read,
                1,
                false,
                AgentRuntimeOutcome::Failed,
                1,
            ),
            (
                OutcomeFirstReply::Text,
                1,
                false,
                AgentRuntimeOutcome::Succeeded,
                1,
            ),
        ] {
            let directory = tempfile::tempdir()?;
            let manager = Arc::new(SessionManager::new(directory.path().to_path_buf()));
            let mut agent = Agent::with_config(AgentRunnerConfig::new(
                Arc::clone(&manager),
                PermissionManager::instance(),
                None,
                GooseMode::Auto,
                true,
                GoosePlatform::GooseCli,
            ));
            let session = manager
                .create_session(
                    directory.path().to_path_buf(),
                    "reply outcome sequence".to_string(),
                    crate::session::session_manager::SessionType::Hidden,
                    GooseMode::Auto,
                )
                .await?;
            let cancellation = CancellationToken::new();
            let provider = Arc::new(OutcomeSequenceProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
                first_reply,
                cancel_on_failure: cancel_after_failure.then(|| cancellation.clone()),
                read_path: directory
                    .path()
                    .join("fixture.txt")
                    .to_string_lossy()
                    .into_owned(),
            });
            if matches!(first_reply, OutcomeFirstReply::Read) {
                std::fs::write(&provider.read_path, "fixture contents")?;
                agent
                    .add_extension(
                        ExtensionConfig::Builtin {
                            name: "developer".to_string(),
                            description: String::new(),
                            display_name: None,
                            timeout: None,
                            bundled: None,
                            available_tools: vec!["read".to_string()],
                        },
                        &session.id,
                    )
                    .await?;
            }
            agent.update_provider(provider.clone(), &session.id).await?;
            agent.after_turn_manager = crate::after_turn::AfterTurnManager::new();
            let mut reply = agent
                .reply(
                    Message::user().with_text("Perform the operation."),
                    SessionConfig {
                        id: session.id,
                        schedule_id: None,
                        max_turns: Some(max_turns),
                        retry_config: None,
                    },
                    Some(cancellation),
                )
                .await?;
            let mut outcome = None;
            let mut successful_tool = false;
            while let Some(event) = reply.next().await {
                match event? {
                    AgentEvent::RuntimeOutcome(observed) => outcome = Some(observed),
                    AgentEvent::Message(message) => {
                        for content in message.content {
                            if let MessageContent::ToolResponse(response) = content {
                                successful_tool |= response
                                    .tool_result
                                    .is_ok_and(|result| !result.is_error.unwrap_or(false));
                            }
                        }
                    }
                    _ => {}
                }
            }
            assert_eq!(outcome, Some(expected));
            assert_eq!(
                provider.calls.load(std::sync::atomic::Ordering::SeqCst),
                expected_calls
            );
            if matches!(first_reply, OutcomeFirstReply::Read) {
                assert!(successful_tool);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn headless_coordinator_hides_and_denies_worker_tools_without_narrowing_workers(
    ) -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let manager = Arc::new(crate::session::SessionManager::new(
            directory.path().to_path_buf(),
        ));
        let config = AgentRunnerConfig::new(
            Arc::clone(&manager),
            PermissionManager::instance(),
            None,
            GooseMode::Auto,
            true,
            GoosePlatform::GooseCli,
        );
        let mut coordinator = Agent::with_config(config.clone());
        coordinator.config.headless_coding_coordinator = true;
        let session = manager
            .create_session(
                directory.path().to_path_buf(),
                "coordinator-tool-scope".to_string(),
                crate::session::session_manager::SessionType::Hidden,
                GooseMode::Auto,
            )
            .await?;
        let developer = || crate::agents::extension::ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: String::new(),
            display_name: None,
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        };
        coordinator.add_extension(developer(), &session.id).await?;
        let coordinator_tools = coordinator.list_tools(&session.id, None).await;
        let coordinator_names: Vec<_> = coordinator_tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect();
        assert!(coordinator_names.contains(&"read"));
        for forbidden in ["edit", "write", "shell", "verify"] {
            assert!(
                !coordinator_names.contains(&forbidden),
                "{forbidden} was exposed"
            );
            let (_, result) = coordinator
                .dispatch_tool_call(
                    CallToolRequestParams::new(forbidden.to_string()),
                    format!("blocked-{forbidden}"),
                    None,
                    &session,
                )
                .await;
            let Err(error) = result else {
                panic!("{forbidden} must be denied before dispatch");
            };
            assert_eq!(error.code, ErrorCode::RESOURCE_NOT_FOUND);
        }

        // A dispatched worker constructs its own Agent; the coordinator's
        // request-only tool scope must not be persisted into inherited grants.
        let worker = Agent::with_config(config);
        worker.add_extension(developer(), &session.id).await?;
        let worker_names: Vec<_> = worker
            .list_tools(&session.id, None)
            .await
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect();
        for authorized in ["edit", "write", "shell", "verify"] {
            assert!(
                worker_names.iter().any(|name| name == authorized),
                "{authorized} missing from worker"
            );
        }
        Ok(())
    }

    /// 2026-09-25: a gpt-5.4-mini worker's turn held only a reasoning item
    /// (the output cap ran out while it thought). That is not the model
    /// finishing, so the loop continues the same session.
    #[test]
    fn a_reasoning_only_turn_continues_the_run() {
        assert_eq!(
            decide_empty_turn(false, false, None, 0),
            EmptyTurnDecision::Continue {
                prompt: EMPTY_TURN_CONTINUATION
            }
        );
        // Cut off by the output cap: the continuation says so.
        assert_eq!(
            decide_empty_turn(false, false, Some("max_output_tokens"), 1),
            EmptyTurnDecision::Continue {
                prompt: EMPTY_TURN_OUTPUT_CAP_CONTINUATION
            }
        );
    }

    #[test]
    fn a_turn_with_text_or_a_tool_call_is_not_empty() {
        assert_eq!(
            decide_empty_turn(true, false, None, 0),
            EmptyTurnDecision::NotEmpty
        );
        assert_eq!(
            decide_empty_turn(false, true, Some("max_output_tokens"), 5),
            EmptyTurnDecision::NotEmpty
        );
    }

    /// Bounded: after the allowed continuations the run ends, and the reason
    /// is stated rather than the loop exiting silently.
    #[test]
    fn empty_turns_stop_continuing_after_the_bound() {
        match decide_empty_turn(
            false,
            false,
            Some("max_output_tokens"),
            MAX_EMPTY_TURN_CONTINUATIONS,
        ) {
            EmptyTurnDecision::GiveUp { reason } => {
                assert!(reason.contains("no text and no tool call"), "{reason}");
                assert!(reason.contains("max_output_tokens"), "{reason}");
            }
            other => panic!("expected GiveUp, got {other:?}"),
        }
    }

    /// The bug this fixes: `mark_error` breaks only the INNER stream loop, so a
    /// turn that failed once, recovered, and completed still reported Failed —
    /// to a coordinator reading a worker's outcome, to the HUD, and (once the
    /// scheduler reads it) to the three-strike auto-pause.
    #[test]
    fn a_failure_latched_then_recovered_reports_succeeded() {
        let mut g = ReplyStateGuard::start();
        g.begin_iteration();
        g.mark_error();
        assert!(
            g.iteration_errored(),
            "the failing iteration must be Failed"
        );

        g.begin_iteration(); // the outer loop goes round and this one succeeds
        assert_eq!(
            runtime_outcome(g.iteration_errored(), false),
            AgentRuntimeOutcome::Succeeded,
            "a turn that recovered must not report Failed"
        );
    }

    /// The other half: the HUD's latch is sticky ON PURPOSE (it must survive the
    /// 2s session-activity derive). This is what stops someone collapsing the
    /// two fields back into one and silently restoring the bug.
    #[test]
    fn the_hud_latch_stays_sticky_across_a_recovered_iteration() {
        let mut g = ReplyStateGuard::start();
        g.begin_iteration();
        g.mark_error();
        g.begin_iteration();
        assert!(
            !g.iteration_errored(),
            "the per-iteration fact is cleared each time round"
        );
        assert!(
            g.errored_for_hud(),
            "the HUD latch must still report the real error that happened"
        );
    }

    #[test]
    fn a_failure_in_the_final_iteration_reports_failed() {
        let mut g = ReplyStateGuard::start();
        g.begin_iteration();
        g.mark_error();
        assert_eq!(
            runtime_outcome(g.iteration_errored(), false),
            AgentRuntimeOutcome::Failed
        );
    }

    /// Cancellation belongs to the scheduler's kill path, which records the
    /// outcome itself. A cancelled run must not ALSO be called Failed, or two
    /// writers race for one fact.
    #[test]
    fn a_cancelled_turn_is_cancelled_not_failed() {
        let mut g = ReplyStateGuard::start();
        g.begin_iteration();
        assert_eq!(
            runtime_outcome(g.iteration_errored(), true),
            AgentRuntimeOutcome::Cancelled
        );
    }

    /// A failure LATCHED earlier must not turn a later cancellation into a
    /// failure either — the last iteration is what the outcome describes.
    #[test]
    fn a_cancellation_after_a_recovered_failure_is_still_cancelled() {
        let mut g = ReplyStateGuard::start();
        g.begin_iteration();
        g.mark_error();
        g.begin_iteration();
        assert_eq!(
            runtime_outcome(g.iteration_errored(), true),
            AgentRuntimeOutcome::Cancelled
        );
    }

    #[test]
    fn a_clean_turn_reports_succeeded() {
        let mut g = ReplyStateGuard::start();
        g.begin_iteration();
        assert_eq!(
            runtime_outcome(g.iteration_errored(), false),
            AgentRuntimeOutcome::Succeeded
        );
    }

    // ── A reasoning + tool-call turn's history (`tool_call_history`) ────

    mod tool_call_history_tests {
        use super::*;
        use crate::providers::formats::openai::{format_messages_with_echo, ReasoningEcho};
        use crate::providers::utils::ImageFormat;

        const STREAM_ID: &str = "chatcmpl-reasoning-turn";
        const REASONING: &str = "Let me check.";

        fn thinking(text: &str) -> MessageContent {
            MessageContent::thinking(text, "")
        }

        fn call(id: &str, path: &str) -> MessageContent {
            MessageContent::tool_request(
                id,
                Ok(CallToolRequestParams::new("read")
                    .with_arguments(rmcp::object!({ "path": path }))),
            )
        }

        /// One frame of a provider reply, carrying the stream's id the way an
        /// OpenAI-compatible chunk does.
        fn frame(content: Vec<MessageContent>) -> Message {
            Message::new(rmcp::model::Role::Assistant, 0, content).with_id(STREAM_ID)
        }

        /// What the reply loop persists for one provider call: a frame with no
        /// tool call is pushed as the no-tool branch pushes it, a frame with
        /// calls goes through `tool_call_history` as the tool branch does (every
        /// parsed call answered), and the turn is coalesced as the end of the
        /// iteration coalesces it.
        fn persist_turn(frames: Vec<Message>) -> Vec<Message> {
            let mut turn = Conversation::default();
            for frame in frames {
                let owned: Vec<ToolRequest> = frame
                    .content
                    .iter()
                    .filter_map(|content| match content {
                        MessageContent::ToolRequest(request) => Some(request.clone()),
                        _ => None,
                    })
                    .collect();
                if owned.is_empty() {
                    turn.push(frame);
                    continue;
                }
                let requests: Vec<&ToolRequest> = owned.iter().collect();
                let mut pending: Vec<Message> = std::mem::take(&mut turn).into_iter().collect();
                let history = tool_call_history(&mut pending, &frame, &requests);
                turn = Conversation::new_unvalidated(pending);
                if let Some(lead) = history.lead {
                    turn.push(lead);
                }
                for (request, message) in requests.iter().zip(history.calls) {
                    if let Some(message) = message {
                        turn.push(message);
                        turn.push(Message::user().with_generated_id().with_tool_response(
                            request.id.clone(),
                            Ok(CallToolResult::success(vec![Content::text("ok")])),
                        ));
                    }
                }
            }
            coalesce_turn_messages(turn.into_iter().collect()).0
        }

        fn carrying<'a>(turn: &'a [Message], call_id: &str) -> &'a Message {
            turn.iter()
                .find(|message| {
                    message.content.iter().any(|content| {
                        matches!(content, MessageContent::ToolRequest(request) if request.id == call_id)
                    })
                })
                .unwrap_or_else(|| panic!("no message carries {call_id}: {turn:?}"))
        }

        fn thinking_of(message: &Message) -> Vec<String> {
            message
                .content
                .iter()
                .filter_map(MessageContent::as_thinking)
                .map(|thinking| thinking.thinking.clone())
                .collect()
        }

        fn all_thinking(turn: &[Message]) -> Vec<String> {
            turn.iter().flat_map(thinking_of).collect()
        }

        fn all_assistant_text(turn: &[Message]) -> String {
            turn.iter()
                .filter(|message| message.role == rmcp::model::Role::Assistant)
                .flat_map(|message| message.content.iter())
                .filter_map(MessageContent::as_text)
                .collect()
        }

        /// The assistant messages an OpenAI-compatible request carries for the
        /// turn, with DeepSeek's always-echo policy.
        fn wire_assistants(turn: &[Message]) -> Vec<Value> {
            format_messages_with_echo(turn, &ImageFormat::OpenAi, ReasoningEcho::Always)
                .into_iter()
                .filter(|message| message["role"] == "assistant")
                .collect()
        }

        /// The traced defect: the deltas, then the tool-call message replaying
        /// them, used to persist "Let me check." three times on call_1 and once
        /// on call_2, so the two calls went out as two assistant messages.
        #[test]
        fn streamed_reasoning_the_tool_call_replays_is_kept_once() {
            let turn = persist_turn(vec![
                frame(vec![thinking("Let me ")]),
                frame(vec![thinking("check.")]),
                frame(vec![
                    thinking(REASONING),
                    call("call_1", "a.rs"),
                    call("call_2", "b.rs"),
                ]),
            ]);

            assert_eq!(thinking_of(carrying(&turn, "call_1")), vec![REASONING]);
            assert_eq!(thinking_of(carrying(&turn, "call_2")), vec![REASONING]);
            assert_eq!(
                all_thinking(&turn),
                vec![REASONING, REASONING],
                "one copy per call and no other: {turn:?}"
            );

            let wire = wire_assistants(&turn);
            assert_eq!(
                wire.len(),
                1,
                "one assistant message for both calls: {wire:?}"
            );
            assert_eq!(wire[0]["tool_calls"].as_array().map(Vec::len), Some(2));
            assert_eq!(wire[0]["reasoning_content"], REASONING);
        }

        /// A non-streamed reply used to persist it twice on call_1.
        #[test]
        fn non_streamed_reasoning_on_the_tool_calls_is_kept_once() {
            let turn = persist_turn(vec![frame(vec![
                thinking(REASONING),
                call("call_1", "a.rs"),
                call("call_2", "b.rs"),
            ])]);

            assert_eq!(thinking_of(carrying(&turn, "call_1")), vec![REASONING]);
            assert_eq!(thinking_of(carrying(&turn, "call_2")), vec![REASONING]);
            assert_eq!(all_thinking(&turn), vec![REASONING, REASONING]);

            let wire = wire_assistants(&turn);
            assert_eq!(wire.len(), 1, "{wire:?}");
            assert_eq!(wire[0]["tool_calls"].as_array().map(Vec::len), Some(2));
            assert_eq!(wire[0]["reasoning_content"], REASONING);
        }

        #[test]
        fn a_single_tool_call_keeps_one_copy_of_its_reasoning() {
            let turn = persist_turn(vec![
                frame(vec![thinking("Let me ")]),
                frame(vec![thinking("check.")]),
                frame(vec![thinking(REASONING), call("call_1", "a.rs")]),
            ]);

            assert_eq!(all_thinking(&turn), vec![REASONING]);
            let wire = wire_assistants(&turn);
            assert_eq!(wire.len(), 1, "{wire:?}");
            assert_eq!(wire[0]["reasoning_content"], REASONING);
        }

        /// Reasoning that arrived while the call's arguments streamed is only in
        /// the replay; the replay wins and nothing is lost.
        #[test]
        fn a_replay_that_extends_the_streamed_reasoning_replaces_it() {
            let turn = persist_turn(vec![
                frame(vec![thinking("Let me ")]),
                frame(vec![thinking(REASONING), call("call_1", "a.rs")]),
            ]);

            assert_eq!(all_thinking(&turn), vec![REASONING]);
        }

        /// A provider that sends NEW reasoning with the call (not a replay)
        /// keeps both.
        #[test]
        fn reasoning_the_stream_did_not_send_is_not_dropped() {
            let turn = persist_turn(vec![
                frame(vec![thinking("Plan: read both.")]),
                frame(vec![
                    thinking("Reading a.rs first."),
                    call("call_1", "a.rs"),
                ]),
            ]);

            let kept = all_thinking(&turn).concat();
            assert!(kept.contains("Plan: read both."), "{kept}");
            assert!(kept.contains("Reading a.rs first."), "{kept}");
        }

        /// The streamed deltas' signature survives the dedupe.
        #[test]
        fn a_replay_without_the_streamed_signature_keeps_it() {
            let turn = persist_turn(vec![
                frame(vec![MessageContent::thinking(REASONING, "sig-streamed")]),
                frame(vec![thinking(REASONING), call("call_1", "a.rs")]),
            ]);

            let carried: Vec<(String, String)> = carrying(&turn, "call_1")
                .content
                .iter()
                .filter_map(MessageContent::as_thinking)
                .map(|thinking| (thinking.thinking.clone(), thinking.signature.clone()))
                .collect();
            assert_eq!(
                carried,
                vec![(REASONING.to_string(), "sig-streamed".to_string())]
            );
        }

        /// Anthropic streams its signed thinking block, then each tool_use as a
        /// response of its own with no thinking in it. The block must stay on
        /// the FIRST tool_use and nowhere else, exactly as before.
        #[test]
        fn anthropic_signed_thinking_stays_on_the_first_tool_use() {
            let turn = persist_turn(vec![
                frame(vec![MessageContent::thinking("plan", "sig-1")]),
                frame(vec![call("toolu_1", "a.rs")]),
                frame(vec![call("toolu_2", "b.rs")]),
            ]);

            let first = carrying(&turn, "toolu_1");
            match first.content.first() {
                Some(MessageContent::Thinking(block)) => {
                    assert_eq!(block.thinking, "plan");
                    assert_eq!(block.signature, "sig-1");
                }
                other => panic!("the signed block must lead the first tool_use: {other:?}"),
            }
            assert!(thinking_of(carrying(&turn, "toolu_2")).is_empty());
            assert_eq!(all_thinking(&turn), vec!["plan"]);
        }

        /// Finding 2: text in the SAME message as the calls used to be dropped.
        #[test]
        fn text_that_came_with_the_tool_calls_is_kept() {
            let turn = persist_turn(vec![frame(vec![
                thinking(REASONING),
                MessageContent::text("I'll read both files."),
                call("call_1", "a.rs"),
                call("call_2", "b.rs"),
            ])]);

            assert_eq!(all_assistant_text(&turn), "I'll read both files.");
            assert_eq!(
                carrying(&turn, "call_1").as_concat_text(),
                "I'll read both files."
            );

            let wire = wire_assistants(&turn);
            assert_eq!(wire.len(), 1, "the text must not block the merge: {wire:?}");
            assert_eq!(wire[0]["content"], "I'll read both files.");
            assert_eq!(wire[0]["tool_calls"].as_array().map(Vec::len), Some(2));
            assert_eq!(wire[0]["reasoning_content"], REASONING);
        }

        /// Ollama's XML tool calls stream their prefix as text and then send it
        /// again with the parsed calls; it is kept once.
        #[test]
        fn text_the_stream_already_sent_is_not_kept_twice() {
            let turn = persist_turn(vec![
                frame(vec![MessageContent::text("I'll check the file. <func")]),
                Message::new(
                    rmcp::model::Role::Assistant,
                    0,
                    vec![
                        MessageContent::text("I'll check the file."),
                        call("call_1", "a.rs"),
                    ],
                ),
            ]);

            let text = all_assistant_text(&turn);
            assert_eq!(text.matches("I'll check the file.").count(), 1, "{text}");
        }

        #[test]
        fn text_that_extends_the_streamed_text_replaces_it() {
            let turn = persist_turn(vec![
                frame(vec![MessageContent::text("I'll check")]),
                frame(vec![
                    MessageContent::text("I'll check the file."),
                    call("call_1", "a.rs"),
                ]),
            ]);

            assert_eq!(all_assistant_text(&turn), "I'll check the file.");
        }

        /// With no call that parsed there is no tool-call message to carry the
        /// reasoning and text, so they keep one of their own, once.
        #[test]
        fn reasoning_and_text_with_no_parsed_call_keep_a_message() {
            let unparsed = MessageContent::tool_request(
                "call_bad",
                Err(ErrorData::new(ErrorCode::INVALID_PARAMS, "bad json", None)),
            );
            let turn = persist_turn(vec![
                frame(vec![thinking("Let me ")]),
                frame(vec![thinking("check.")]),
                frame(vec![
                    thinking(REASONING),
                    MessageContent::text("Reading."),
                    unparsed,
                ]),
            ]);

            assert_eq!(all_thinking(&turn), vec![REASONING], "{turn:?}");
            assert_eq!(all_assistant_text(&turn), "Reading.");
            assert!(
                turn.iter().all(|message| !message.is_tool_call()),
                "an unparsed call is answered by a recovery observation, not kept: {turn:?}"
            );
        }
    }

    // ── Y-poll-suspend ───────────────────────────────────────────────────

    /// Pure `DispatchPollGuard` logic — no DB, no provider, no async: the
    /// rule itself, isolated from the machinery that carries it out.
    mod dispatch_poll_guard_tests {
        use super::*;
        use crate::goal_state::GoalState;
        use std::collections::BTreeMap;

        fn snapshot(state: GoalState) -> BTreeMap<String, GoalState> {
            let mut map = BTreeMap::new();
            map.insert("card-1".to_string(), state);
            map
        }

        /// The rule this guard exists to enforce (see the module header
        /// comment above `DispatchPollGuard` for the measured evidence): one
        /// inspection turn against unchanged dispatched-worker state is
        /// free — the coordinator is not blind — but a SECOND consecutive
        /// turn that only re-reads the identical state must suspend.
        #[test]
        fn a_second_consecutive_unchanged_inspection_turn_suspends_but_the_first_does_not() {
            let mut guard = DispatchPollGuard::default();
            let snap = snapshot(GoalState::InProgress);
            let tools = vec!["orchestrator__view_session".to_string()];

            assert!(
                !guard.observe_turn(&tools, Some(snap.clone())),
                "the first look since dispatch is always free"
            );
            assert!(
                guard.observe_turn(&tools, Some(snap)),
                "a second look at the exact same state must suspend"
            );
        }

        /// An actionable tool call (dispatch, steer, escalate, message, ...)
        /// is progress, not polling: it never suspends, and it resets the
        /// guard's memory so the very next inspection is a fresh free look
        /// again rather than inheriting a stale "already looked once".
        #[test]
        fn an_actionable_tool_call_resets_the_guard_even_if_state_is_unchanged() {
            let mut guard = DispatchPollGuard::default();
            let snap = snapshot(GoalState::InProgress);
            let inspect = vec!["view_session".to_string()];
            let act = vec!["goal_advance".to_string()];

            assert!(!guard.observe_turn(&inspect, Some(snap.clone())));
            assert!(
                !guard.observe_turn(&act, Some(snap.clone())),
                "an actionable turn is never suspended"
            );
            assert!(
                !guard.observe_turn(&inspect, Some(snap.clone())),
                "the turn right after an actionable one is a fresh free look"
            );
            assert!(
                guard.observe_turn(&inspect, Some(snap)),
                "only the SECOND consecutive unchanged inspection after that suspends"
            );
        }

        /// A worker that actually moved between two inspections is new
        /// information, not a repeated look — it grants another free
        /// inspection rather than counting toward the suspend threshold.
        #[test]
        fn an_observed_state_change_grants_a_fresh_free_look() {
            let mut guard = DispatchPollGuard::default();
            let inspect = vec!["goal_status".to_string()];

            assert!(!guard.observe_turn(&inspect, Some(snapshot(GoalState::Ready))));
            assert!(
                !guard.observe_turn(&inspect, Some(snapshot(GoalState::InProgress))),
                "state changed since the last look — this is a free look, not turn 2 of a pair"
            );
            assert!(
                guard.observe_turn(&inspect, Some(snapshot(GoalState::InProgress))),
                "NOW it is a second consecutive look at the same (InProgress) state"
            );
        }

        /// Nothing dispatched means nothing to be blind about — the guard
        /// must never suspend a session with no dispatched worker, no matter
        /// how many inspection-only turns it takes in a row.
        #[test]
        fn nothing_dispatched_never_suspends_and_forgets_its_own_history() {
            let mut guard = DispatchPollGuard::default();
            let inspect = vec!["list_sessions".to_string()];
            assert!(!guard.observe_turn(&inspect, Some(snapshot(GoalState::InProgress))));
            assert!(!guard.observe_turn(&inspect, None));
            assert!(!guard.observe_turn(&inspect, None));
        }

        /// A turn that mixes one inspection call with one actionable call is
        /// never treated as inspection-only, even though most of its tool
        /// calls were reads.
        #[test]
        fn a_mixed_turn_with_one_actionable_call_is_never_treated_as_inspection_only() {
            let mut guard = DispatchPollGuard::default();
            let snap = snapshot(GoalState::Review);
            let mixed = vec!["view_session".to_string(), "steer_goal".to_string()];
            assert!(!guard.observe_turn(&mixed, Some(snap.clone())));
            assert!(!guard.observe_turn(&mixed, Some(snap)));
        }

        /// Tool names arrive prefixed with their owning extension
        /// (`extension_manager`'s own `"{extension}__{tool}"` convention) —
        /// the allowlist must match on the base name either way.
        #[test]
        fn tool_names_with_an_extension_prefix_are_matched_on_their_base_name() {
            assert!(is_inspection_only_tool("orchestrator__view_session"));
            assert!(is_inspection_only_tool("view_session"));
            assert!(!is_inspection_only_tool("orchestrator__goal_advance"));
        }
    }

    /// `suspend_for_dispatched_goals` against a real (temp-file) SQLite pool
    /// — the same fixture shape `orchestrator.rs`'s own drain tests use
    /// (`SessionManager::new` over a tempdir, a coordinator + a fake
    /// `SubAgent` child session, a goal card moved between board columns to
    /// change its `GoalState`).
    mod suspend_for_dispatched_goals_tests {
        use super::*;
        use crate::session::session_manager::SessionType;

        /// What this module can and cannot prove.
        ///
        /// The waiting itself belongs to `drain_dispatched_goals`, which has
        /// its own fixtures and tests in orchestrator.rs; reproducing a
        /// worker that genuinely reads as "still running" here needs an
        /// execution receipt the drain checks
        /// (`worker_session_execution_receipt_is_terminal`), and a fixture
        /// that fakes one would prove the fake, not the behaviour. So these
        /// cover the seam this module actually owns — the decision to
        /// suspend, the cancellation contract, and the shape of what comes
        /// back — and the drain's own tests cover the wait.
        #[tokio::test]
        async fn an_already_cancelled_token_returns_none_without_draining() {
            let tmp = tempfile::tempdir().unwrap();
            let sm = Arc::new(crate::session::SessionManager::new(
                tmp.path().to_path_buf(),
            ));
            let pool = sm.pool_clone().await.unwrap();
            let coordinator = sm
                .create_session(
                    tmp.path().to_path_buf(),
                    "coordinator".to_string(),
                    SessionType::Hidden,
                    GooseMode::Auto,
                )
                .await
                .unwrap();
            let token = CancellationToken::new();
            token.cancel();
            let start = std::time::Instant::now();
            let notice = suspend_for_dispatched_goals(
                &pool,
                &coordinator.id,
                std::time::Duration::from_secs(30),
                &Some(token),
            )
            .await;
            assert!(notice.is_none(), "a cancelled wait resumes nothing");
            assert!(
                start.elapsed() < std::time::Duration::from_secs(1),
                "cancellation must not sleep out the cap: {:?}",
                start.elapsed()
            );
        }

        /// The other half of the saving: what the model is handed on resume is
        /// a few lines of state change, never the worker's transcript. The
        /// coordinator's context grew 26k -> 53k tokens in the reported run
        /// precisely because it kept reading transcripts.
        #[test]
        fn the_resume_notice_is_compact_and_names_what_changed() {
            let notice = format_compact_resume_notice(
                &[
                    (
                        "20260920_6".to_string(),
                        crate::goal_state::GoalState::InProgress,
                    ),
                    (
                        "20260920_6".to_string(),
                        crate::goal_state::GoalState::Complete,
                    ),
                ],
                crate::agents::platform_extensions::orchestrator::DrainSummary {
                    finished: 1,
                    timed_out: 0,
                    still_running: 0,
                },
            );
            assert!(notice.contains("20260920_6"), "names the worker: {notice}");
            assert!(notice.contains("complete"), "names the new state: {notice}");
            assert!(
                notice.lines().count() <= 6,
                "a resume notice is a few lines, not a transcript: {notice}"
            );
            assert!(
                notice.len() < 600,
                "a resume notice must stay compact: {} chars",
                notice.len()
            );
        }

        /// QA 2026-09-25: the suspension reaches its cap with a worker still
        /// going and parks nothing (`ReturnOnExpiry`), so the notice must say
        /// the worker is still running — the old "N parked at the wait cap"
        /// line told the coordinator its worker had been stopped.
        #[test]
        fn the_resume_notice_says_still_running_when_nothing_was_parked() {
            let notice = format_compact_resume_notice(
                &[],
                crate::agents::platform_extensions::orchestrator::DrainSummary {
                    finished: 0,
                    timed_out: 0,
                    still_running: 1,
                },
            );
            assert!(notice.contains("1 still running"), "{notice}");
            assert!(!notice.contains("parked"), "nothing was parked: {notice}");

            let stalled = format_compact_resume_notice(
                &[],
                crate::agents::platform_extensions::orchestrator::DrainSummary {
                    finished: 0,
                    timed_out: 1,
                    still_running: 0,
                },
            );
            assert!(
                stalled.contains("1 parked") && !stalled.contains("wait cap."),
                "a genuine stall park is named as such, never as a cap park: {stalled}"
            );
        }

        /// The mid-turn cap has its own key: the post-run drain's
        /// `PERMAGENT_HEADLESS_DRAIN_SECS` (raised to 900s as a stopgap on
        /// 2026-09-25) must not retune how long a coordinator sleeps.
        #[test]
        fn the_mid_turn_cap_is_not_the_post_run_drain_key() {
            assert_ne!(
                MID_TURN_DISPATCH_SUSPEND_SECS_KEY,
                "PERMAGENT_HEADLESS_DRAIN_SECS"
            );
            assert_eq!(DEFAULT_MID_TURN_DISPATCH_SUSPEND_CAP_SECS, 240);
        }
    }
}
