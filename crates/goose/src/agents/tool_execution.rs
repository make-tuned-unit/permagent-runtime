use async_stream::try_stream;
use futures::stream::{self, BoxStream};
use futures::{Stream, StreamExt};
use rmcp::model::CallToolResult;
use std::collections::HashMap;
use std::future::Future;
use tokio_util::sync::CancellationToken;

use std::path::PathBuf;

use crate::config::permission::PermissionLevel;
use crate::mcp_utils::ToolResult;
use crate::permission::Permission;
use rmcp::model::{Content, ServerNotification};

/// Context passed through the tool call dispatch chain.
pub struct ToolCallContext {
    pub session_id: String,
    pub working_dir: Option<PathBuf>,
    pub tool_call_request_id: Option<String>,
    pub model: Option<String>,
}

impl ToolCallContext {
    pub fn new(
        session_id: String,
        working_dir: Option<PathBuf>,
        tool_call_request_id: Option<String>,
    ) -> Self {
        Self {
            session_id,
            working_dir,
            tool_call_request_id,
            model: None,
        }
    }

    pub fn with_model(mut self, model: Option<String>) -> Self {
        self.model = model;
        self
    }

    pub fn working_dir_str(&self) -> Option<&str> {
        self.working_dir.as_ref().and_then(|p| p.to_str())
    }
}

// ToolCallResult combines the result of a tool call with an optional notification stream that
// can be used to receive notifications from the tool.
pub struct ToolCallResult {
    pub result: Box<dyn Future<Output = ToolResult<rmcp::model::CallToolResult>> + Send + Unpin>,
    pub notification_stream: Option<Box<dyn Stream<Item = ServerNotification> + Send + Unpin>>,
}

impl From<ToolResult<rmcp::model::CallToolResult>> for ToolCallResult {
    fn from(result: ToolResult<rmcp::model::CallToolResult>) -> Self {
        Self {
            result: Box::new(futures::future::ready(result)),
            notification_stream: None,
        }
    }
}

use super::agent::{tool_stream, ToolStream};
use crate::agents::Agent;
use crate::conversation::message::{ActionRequiredData, Message, MessageContent, ToolRequest};
use crate::permission::permission_confirmation::PrincipalType;
use crate::permission::PermissionConfirmation;
use crate::session::Session;
use crate::tool_inspection::get_security_finding_id_from_results;

pub const DECLINED_RESPONSE: &str = "The user has declined to run this tool. \
    DO NOT attempt to call this tool again. \
    If there are no alternative methods to proceed, clearly explain the situation and STOP.";

/// Tool response recorded when a HEADLESS agent (scheduled-recipe job — no
/// interactive approver exists) hits a tool call that would require approval.
/// The call is auto-denied instead of parking: a park could only hang the job
/// forever, and silently widening to auto-approval would let anything able to
/// schedule a recipe bypass approve mode. Already always-allowed tools are
/// unaffected (they never reach the approval path).
pub const HEADLESS_SKIPPED_RESPONSE: &str =
    "This tool call requires interactive approval, but this session runs headless \
    (a scheduled job with no approver) — the call was NOT run. \
    DO NOT attempt to call this tool again. \
    Continue with what can be done without it and clearly report this skipped step \
    in your final output. The user can pre-approve the tool (Always Allow) or run \
    the recipe in auto mode to permit it.";

pub const CHAT_MODE_TOOL_SKIPPED_RESPONSE: &str = "Let the user know the tool call was skipped in goose chat mode. \
                                        DO NOT apologize for skipping the tool call. DO NOT say sorry. \
                                        Provide an explanation of what the tool call would do, structured as a \
                                        plan for the user. Again, DO NOT apologize. \
                                        **Example Plan:**\n \
                                        1. **Identify Task Scope** - Determine the purpose and expected outcome.\n \
                                        2. **Outline Steps** - Break down the steps.\n \
                                        If needed, adjust the explanation based on user preferences or questions.";

impl Agent {
    pub(crate) fn handle_approval_tool_requests<'a>(
        &'a self,
        tool_requests: &'a [ToolRequest],
        tool_futures: &'a mut Vec<(String, ToolStream)>,
        request_to_response_map: &'a mut HashMap<String, Message>,
        cancellation_token: Option<CancellationToken>,
        session: &'a Session,
        inspection_results: &'a [crate::tool_inspection::InspectionResult],
    ) -> BoxStream<'a, anyhow::Result<Message>> {
        try_stream! {
        for request in tool_requests.iter() {
            if let Ok(tool_call) = request.tool_call.clone() {
                // HEADLESS agents (scheduled jobs) must never park: no approver
                // exists, so a park is a permanent hang and a filed decision an
                // unanswerable zombie card. Auto-deny with a recorded skip —
                // before the router registration, the inbox filing, and the
                // action_required yield. DenyOnce semantics: no NeverAllow is
                // persisted (the denial is circumstantial, not user intent).
                if self.is_headless() {
                    tracing::info!(
                        tool_name = %tool_call.name,
                        tool_request_id = %request.id,
                        session_id = %session.id,
                        "headless session: approval-required tool auto-denied (recorded skip)"
                    );
                    if let Some(response) = request_to_response_map.get_mut(&request.id) {
                        response.add_tool_response_with_metadata(
                            request.id.clone(),
                            Ok(CallToolResult::error(vec![Content::text(
                                HEADLESS_SKIPPED_RESPONSE,
                            )])),
                            request.metadata.as_ref(),
                        );
                    }
                    continue;
                }

                let security_message = inspection_results.iter()
                    .find(|result| result.tool_request_id == request.id)
                    .and_then(|result| {
                        if let crate::tool_inspection::InspectionAction::RequireApproval(Some(message)) = &result.action {
                            Some(message.clone())
                        } else {
                            None
                        }
                    });

                let confirmation_rx = self
                    .tool_confirmation_router
                    .register(request.id.clone())
                    .await?;

                let tool_name = tool_call.name.to_string();

                // Route this needs-approval tool call through the Decision Inbox so the
                // command-center (which never calls /action-required/tool-confirmation)
                // can answer it: answering the decision approve/reject delivers the
                // confirmation back to `confirmation_rx` below via
                // ToolConfirmationRouter::deliver, unblocking this parked await.
                // Best-effort — a failure here leaves the legacy action_required path
                // (yielded just below) intact and never breaks the turn.
                self.create_tool_approval_decision(
                    &request.id,
                    &tool_name,
                    serde_json::Value::Object(tool_call.arguments.clone().unwrap_or_default()),
                    security_message.as_deref(),
                    &session.id,
                )
                .await;

                let action_required_msg = Message::assistant()
                    .with_action_required(
                        request.id.clone(),
                        tool_name.clone(),
                        tool_call.arguments.clone().unwrap_or_default(),
                        security_message,
                    )
                    .user_only();
                yield action_required_msg;

                let confirmation = confirmation_rx.await
                    .map_err(|_| anyhow::anyhow!("Confirmation channel closed for request {}", request.id))?;

                if let Some(finding_id) = get_security_finding_id_from_results(&request.id, inspection_results) {
                    tracing::info!(
                        monotonic_counter.goose.prompt_injection_user_decisions = 1,
                        decision = ?confirmation.permission,
                        finding_id = %finding_id,
                        tool_request_id = %request.id,
                        "Prompt injection detection: user decision on command injection finding"
                    );
                }

                if confirmation.permission == Permission::AllowOnce || confirmation.permission == Permission::AlwaysAllow {
                    let (req_id, tool_result) = self.dispatch_tool_call(tool_call.clone(), request.id.clone(), cancellation_token.clone(), session).await;

                    tool_futures.push((req_id, match tool_result {
                        Ok(result) => tool_stream(
                            result.notification_stream.unwrap_or_else(|| Box::new(stream::empty())),
                            result.result,
                        ),
                        Err(e) => tool_stream(
                            Box::new(stream::empty()),
                            futures::future::ready(Err(e)),
                        ),
                    }));

                    if confirmation.permission == Permission::AlwaysAllow {
                        self.tool_inspection_manager
                            .update_permission_manager(&tool_call.name, PermissionLevel::AlwaysAllow)
                            .await;
                    }
                } else {
                    if let Some(response) = request_to_response_map.get_mut(&request.id) {
                        response.add_tool_response_with_metadata(
                            request.id.clone(),
                            Ok(CallToolResult::error(vec![Content::text(DECLINED_RESPONSE)])),
                            request.metadata.as_ref(),
                        );
                    }

                    if confirmation.permission == Permission::AlwaysDeny {
                        self.tool_inspection_manager
                            .update_permission_manager(&tool_call.name, PermissionLevel::NeverAllow)
                            .await;
                    }
                }
            }
        }
    }.boxed()
    }

    pub(crate) fn handle_frontend_tool_request<'a>(
        &'a self,
        tool_request: &'a ToolRequest,
        message_tool_response: &'a mut Message,
    ) -> BoxStream<'a, anyhow::Result<Message>> {
        try_stream! {
                if let Ok(tool_call) = tool_request.tool_call.clone() {
                    if self.is_frontend_tool(&tool_call.name).await {
                        yield Message::assistant().with_frontend_tool_request(
                            tool_request.id.clone(),
                            Ok(tool_call.clone())
                        );

                        if let Some((id, result)) = self.tool_result_rx.lock().await.recv().await {
                            message_tool_response.add_tool_response_with_metadata(
                                id,
                                result,
                                tool_request.metadata.as_ref(),
                            );
                        }
                    }
            }
        }
        .boxed()
    }

    /// Surface a needs-approval tool call as a Decision-Inbox row (`kind =
    /// tool_approval`) so it can be answered asynchronously by the command-center.
    /// The row carries the `session_id` + `request_id` routing keys; answering it
    /// approve/reject delivers the confirmation back to the parked
    /// [`ToolConfirmationRouter`] await that `handle_approval_tool_requests` is
    /// suspended on (see `goose-server` `routes::decisions::deliver_tool_confirmation`).
    /// Best-effort: any failure is logged and the turn proceeds on the legacy
    /// `/action-required` path, so this never breaks a turn.
    async fn create_tool_approval_decision(
        &self,
        request_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        security_message: Option<&str>,
        session_id: &str,
    ) {
        use crate::decisions::{
            create_decision, NewDecision, ToolApprovalPayload, MAX_HEADLINE_CHARS,
        };

        // Population gate: only file when answering can actually reach the
        // parked waiter. A card that cannot be delivered is worse than no card
        // — the user "answers" it and nothing happens.
        if self.is_headless() {
            // Headless agents never park (auto-denied upstream); belt and
            // braces here so no call path can file an unanswerable card.
            tracing::debug!(
                "tool-approval decision skipped for request {}: headless agent",
                request_id
            );
            return;
        }
        if !crate::decisions::process_serves_inbox() {
            // This process does not serve the Decision Inbox answer path
            // (CLI session, example, one-off binary). Its parks are answered
            // on its own surface — the CLI terminal prompt via
            // `Agent::handle_confirmation` — and a row filed from here would
            // be a zombie card in the daemon's inbox: same DB, but the answer
            // path resolves agents in the DAEMON process and can never reach
            // this waiter. The park itself proceeds unchanged.
            tracing::debug!(
                "tool-approval decision skipped for request {}: process does not serve the inbox",
                request_id
            );
            return;
        }

        let pool = match self.config.session_manager.pool_clone().await {
            Ok(pool) => pool,
            Err(e) => {
                tracing::warn!(
                    "tool-approval decision skipped for request {}: no DB pool ({})",
                    request_id,
                    e
                );
                return;
            }
        };

        // Compact args preview for the human-readable detail; the full arguments
        // live in the payload (borrow before `arguments` is moved into it). A
        // clipped preview carries an explicit truncation marker — a dangerous
        // tail past the cap must never be silently invisible at approval time
        // (the inbox card offers the full payload arguments alongside).
        let args_preview = args_preview(
            &serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string()),
            ARGS_PREVIEW_MAX_CHARS,
        );

        // Headline must be <= MAX_HEADLINE_CHARS or create_decision stores the row
        // as malformed; cap defensively (tool names are short in practice).
        let headline = {
            let h = format!("Approve tool call: {}", tool_name);
            if h.chars().count() > MAX_HEADLINE_CHARS {
                h.chars().take(MAX_HEADLINE_CHARS).collect()
            } else {
                h
            }
        };
        let mut detail = format!(
            "The assistant is requesting approval to run the '{}' tool with arguments: {}",
            tool_name, args_preview
        );
        if let Some(msg) = security_message {
            detail.push_str(&format!(" — security note: {}", msg));
        }

        let payload = ToolApprovalPayload {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments,
            security_message: security_message.map(str::to_string),
        };
        let payload_json = match serde_json::to_value(&payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "tool-approval decision skipped for request {}: payload serialize failed ({})",
                    request_id,
                    e
                );
                return;
            }
        };

        if let Err(e) = create_decision(
            &pool,
            NewDecision {
                kind: "tool_approval".to_string(),
                headline: Some(headline),
                detail: Some(detail),
                payload: payload_json,
                ..Default::default()
            },
        )
        .await
        {
            tracing::warn!(
                "tool-approval decision create failed for request {}: {}",
                request_id,
                e
            );
        }
    }

    /// Bridge PROVIDER-side permission parks into the Decision Inbox.
    ///
    /// Providers with `PermissionRouting::ActionRequired` (claude-code, ACP)
    /// run their own permission loop: mid-stream they yield an ActionRequired
    /// `ToolConfirmation` message and park on an internal oneshot keyed by the
    /// request id, waiting for `Provider::handle_permission_confirmation`.
    /// That path bypasses `handle_approval_tool_requests`, so without this
    /// bridge no decision is filed and approve-mode coding sessions park with
    /// an EMPTY inbox — the trust-chain hole this closes.
    ///
    /// Called by the reply loop on every provider-stream message, BEFORE the
    /// message is yielded to the client (same file-then-surface order as the
    /// core park). The provider registers its oneshot before yielding, so by
    /// the time this runs the park is already answerable:
    ///
    /// - normal agents: file a `tool_approval` row (best-effort — a filing
    ///   failure never breaks the turn; the legacy action_required surface
    ///   still flows). Answering the row delivers through
    ///   `Agent::handle_confirmation` → `handle_permission_confirmation` →
    ///   the provider's oneshot, and the delivered/no-waiter bool stays
    ///   honest end to end.
    /// - headless agents (scheduled jobs — no approver exists): answer the
    ///   park immediately ourselves with a deny (recorded skip). Never parks
    ///   the job, never files a card nobody can act on. The subprocess
    ///   continues with the tool denied.
    ///
    /// Non-ToolConfirmation ActionRequired content (elicitations) is not a
    /// permission park and passes through untouched.
    pub(crate) async fn bridge_provider_action_required(
        &self,
        response: &Message,
        session_id: &str,
    ) {
        for content in &response.content {
            let MessageContent::ActionRequired(action_required) = content else {
                continue;
            };
            let ActionRequiredData::ToolConfirmation {
                id,
                tool_name,
                arguments,
                // ToolConfirmation.prompt is provider-controlled text (ACP puts
                // tool CONTENT there; the core park puts the inspector's
                // security note). Untrusted text must not wear the trusted
                // "security note" label on an approval card, so it is NOT
                // forwarded into the decision detail — the full arguments in
                // the payload carry the reviewable substance.
                prompt: _,
            } = &action_required.data
            else {
                continue;
            };

            if self.is_headless() {
                let delivered = self
                    .handle_confirmation(
                        id.clone(),
                        PermissionConfirmation {
                            principal_type: PrincipalType::Tool,
                            permission: Permission::DenyOnce,
                        },
                    )
                    .await;
                tracing::info!(
                    tool_name = %tool_name,
                    tool_request_id = %id,
                    session_id = %session_id,
                    delivered,
                    "headless session: provider-side approval park auto-denied (recorded skip)"
                );
                continue;
            }

            self.create_tool_approval_decision(
                id,
                tool_name,
                serde_json::Value::Object(arguments.clone()),
                None,
                session_id,
            )
            .await;
        }
    }
}

/// Character cap for the tool-argument preview embedded in a `tool_approval`
/// decision's `detail` text.
const ARGS_PREVIEW_MAX_CHARS: usize = 400;

/// Clip a JSON args string to `max_chars` characters for the human-readable
/// decision detail. Informed consent requires that clipping is EXPLICIT: a
/// clipped preview ends with "… [truncated — N more chars]" so the approver
/// knows there is a tail they have not seen (and can open the full arguments
/// on the card). Unclipped input is returned verbatim, marker-free.
fn args_preview(args_json: &str, max_chars: usize) -> String {
    let total_chars = args_json.chars().count();
    if total_chars <= max_chars {
        return args_json.to_string();
    }
    let clipped: String = args_json.chars().take(max_chars).collect();
    format!(
        "{}… [truncated — {} more chars]",
        clipped,
        total_chars - max_chars
    )
}

#[cfg(test)]
mod tests {
    use super::args_preview;

    #[test]
    fn args_preview_untouched_at_or_under_cap() {
        assert_eq!(args_preview("", 400), "");
        let exactly = "x".repeat(400);
        let out = args_preview(&exactly, 400);
        assert_eq!(out, exactly, "exactly-at-cap input must not be marked");
        assert!(!out.contains("truncated"));
    }

    #[test]
    fn args_preview_marks_exactly_when_clipped() {
        let over = "x".repeat(401);
        let out = args_preview(&over, 400);
        assert!(
            out.ends_with("… [truncated — 1 more chars]"),
            "one char past the cap must be marked: {}",
            out
        );
        assert!(out.starts_with(&"x".repeat(400)));

        let far_over = "y".repeat(1000);
        let out = args_preview(&far_over, 400);
        assert!(out.ends_with("… [truncated — 600 more chars]"), "{}", out);
    }

    #[test]
    fn args_preview_counts_chars_not_bytes() {
        // 500 multibyte chars (3 bytes each in UTF-8): clips at 400 CHARS on a
        // valid boundary and reports the remaining 100 chars.
        let multibyte = "€".repeat(500);
        let out = args_preview(&multibyte, 400);
        assert!(out.starts_with(&"€".repeat(400)));
        assert!(out.ends_with("… [truncated — 100 more chars]"), "{}", out);
    }

    // ── Park-bridge population gating ───────────────────────────────────────
    //
    // INVARIANT for every test below: no `permagent` LIB unit test may ever
    // call `decisions::mark_process_serves_inbox()` — the flag is process-wide
    // and irreversible, and these tests share one test binary. The unset flag
    // models exactly the CLI-process population. The flag-SET behavior (real
    // daemon filing + inbox answer round trip) lives in the integration binary
    // `tests/park_bridge.rs`, which gets its own process.

    use super::*;
    use crate::agents::{Agent, AgentRunnerConfig, GoosePlatform};
    use crate::config::permission::PermissionManager;
    use crate::config::GooseMode;
    use crate::conversation::message::MessageContent;
    use crate::permission::permission_confirmation::PrincipalType;
    use crate::permission::{Permission, PermissionConfirmation};
    use crate::providers::base::PermissionRouting;
    use crate::session::{Session, SessionManager, SessionType};
    use futures::TryStreamExt;
    use rmcp::model::CallToolRequestParams;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    /// Isolated agent + session + pool (no globals beyond PermissionManager,
    /// which is read-only here and already exercised by manager.rs tests).
    async fn park_test_agent(
        tmp: &tempfile::TempDir,
        mode: GooseMode,
    ) -> (Agent, Session, crate::sqlx::Pool<crate::sqlx::Sqlite>) {
        let session_manager = Arc::new(SessionManager::new(tmp.path().to_path_buf()));
        let agent = Agent::with_config(AgentRunnerConfig::new(
            Arc::clone(&session_manager),
            PermissionManager::instance(),
            None,
            mode,
            true,
            GoosePlatform::GooseCli,
        ));
        let session = session_manager
            .create_session(
                tmp.path().to_path_buf(),
                "park-bridge test".to_string(),
                SessionType::User,
                mode,
            )
            .await
            .expect("create session");
        let pool = session_manager.pool_clone().await.expect("pool");
        (agent, session, pool)
    }

    fn approval_tool_request(id: &str, tool: &str) -> ToolRequest {
        let mut args = rmcp::model::JsonObject::new();
        args.insert("command".to_string(), serde_json::json!("rm -rf /tmp/x"));
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams::new(tool.to_string()).with_arguments(args)),
            metadata: None,
            tool_meta: None,
        }
    }

    async fn open_decision_exists(
        pool: &crate::sqlx::Pool<crate::sqlx::Sqlite>,
        request_id: &str,
    ) -> bool {
        crate::decisions::find_open_tool_approval_by_request_id(pool, request_id)
            .await
            .expect("decision lookup")
            .is_some()
    }

    /// Concatenated text of every ToolResponse content block in `message`
    /// (`as_concat_text` covers only Text content, not tool results).
    fn tool_response_text(message: &Message) -> String {
        message
            .content
            .iter()
            .filter_map(|c| match c {
                MessageContent::ToolResponse(r) => r.tool_result.as_ref().ok(),
                _ => None,
            })
            .flat_map(|result| &result.content)
            .filter_map(|content| match &content.raw {
                rmcp::model::RawContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A provider parked exactly the way claude-code/ACP park: a oneshot per
    /// request id, consumed by `handle_permission_confirmation`.
    struct ParkedProvider {
        pending: tokio::sync::Mutex<HashMap<String, oneshot::Sender<PermissionConfirmation>>>,
    }

    impl ParkedProvider {
        fn new() -> Self {
            Self {
                pending: tokio::sync::Mutex::new(HashMap::new()),
            }
        }
    }

    impl std::fmt::Debug for ParkedProvider {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ParkedProvider").finish()
        }
    }

    #[async_trait::async_trait]
    impl crate::providers::base::Provider for ParkedProvider {
        fn get_name(&self) -> &str {
            "test-parked"
        }
        fn get_model_config(&self) -> crate::model::ModelConfig {
            crate::model::ModelConfig::new("test").unwrap()
        }
        async fn stream(
            &self,
            _: &crate::model::ModelConfig,
            _: &str,
            _: &str,
            _: &[Message],
            _: &[rmcp::model::Tool],
        ) -> Result<crate::providers::base::MessageStream, crate::providers::errors::ProviderError>
        {
            unimplemented!("not driven in these tests")
        }
        fn permission_routing(&self) -> PermissionRouting {
            PermissionRouting::ActionRequired
        }
        async fn handle_permission_confirmation(
            &self,
            request_id: &str,
            confirmation: &PermissionConfirmation,
        ) -> bool {
            let mut pending = self.pending.lock().await;
            if let Some(tx) = pending.remove(request_id) {
                let _ = tx.send(confirmation.clone());
                return true;
            }
            false
        }
    }

    fn provider_action_required(request_id: &str, tool: &str) -> Message {
        let mut args = rmcp::model::JsonObject::new();
        args.insert("path".to_string(), serde_json::json!("/tmp/f.txt"));
        Message::assistant().with_action_required(
            request_id.to_string(),
            tool.to_string(),
            args,
            None,
        )
    }

    /// Headless core park: the approval-required request is auto-denied with a
    /// recorded skip — no park (the stream completes without yielding an
    /// action_required and without any confirmation being delivered), no
    /// decision row, no dispatched tool.
    #[tokio::test]
    async fn headless_core_park_auto_denies_without_parking_or_filing() {
        let tmp = tempfile::tempdir().unwrap();
        let (agent, session, pool) = park_test_agent(&tmp, GooseMode::Approve).await;
        agent.set_headless(true);

        let request = approval_tool_request("req-headless-core", "write_file");
        let mut tool_futures = Vec::new();
        let mut response_map = HashMap::new();
        response_map.insert(
            "req-headless-core".to_string(),
            Message::user().with_generated_id(),
        );

        {
            let mut stream = agent.handle_approval_tool_requests(
                std::slice::from_ref(&request),
                &mut tool_futures,
                &mut response_map,
                None,
                &session,
                &[],
            );
            assert!(
                stream.try_next().await.expect("stream").is_none(),
                "headless skip must not yield an action_required park message"
            );
        }

        assert!(
            tool_futures.is_empty(),
            "denied tool must not be dispatched"
        );
        let recorded = tool_response_text(
            response_map
                .get("req-headless-core")
                .expect("response entry"),
        );
        assert!(
            recorded.contains("runs headless"),
            "skip must be recorded for the model: {}",
            recorded
        );
        assert!(
            !open_decision_exists(&pool, "req-headless-core").await,
            "headless agents must never file a tool_approval decision"
        );
    }

    /// CLI-population core park (process does not serve the inbox): the park
    /// itself is UNCHANGED — action_required is yielded and the confirmation
    /// router delivers the terminal's answer — but no decision row is filed.
    #[tokio::test]
    async fn unregistered_process_core_park_parks_but_files_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (agent, session, pool) = park_test_agent(&tmp, GooseMode::Approve).await;

        let request = approval_tool_request("req-cli-core", "write_file");
        let mut tool_futures = Vec::new();
        let mut response_map = HashMap::new();
        response_map.insert(
            "req-cli-core".to_string(),
            Message::user().with_generated_id(),
        );

        {
            let mut stream = agent.handle_approval_tool_requests(
                std::slice::from_ref(&request),
                &mut tool_futures,
                &mut response_map,
                None,
                &session,
                &[],
            );

            let parked = stream
                .try_next()
                .await
                .expect("stream")
                .expect("action_required must still be yielded for the CLI prompt");
            assert!(
                parked
                    .content
                    .iter()
                    .any(|c| matches!(c, MessageContent::ActionRequired(_))),
                "CLI park surface must be unchanged"
            );

            assert!(
                !open_decision_exists(&pool, "req-cli-core").await,
                "a process that does not serve the inbox must not file (zombie card)"
            );

            // The CLI terminal answers through the same Agent API it uses today.
            let delivered = agent
                .handle_confirmation(
                    "req-cli-core".to_string(),
                    PermissionConfirmation {
                        principal_type: PrincipalType::Tool,
                        permission: Permission::DenyOnce,
                    },
                )
                .await;
            assert!(delivered, "router must deliver to the parked await");

            assert!(
                stream.try_next().await.expect("stream").is_none(),
                "stream must complete after the deny"
            );
        }

        let recorded =
            tool_response_text(response_map.get("req-cli-core").expect("response entry"));
        assert!(
            recorded.contains("declined"),
            "deny must record the declined response: {}",
            recorded
        );
        assert!(tool_futures.is_empty());
    }

    /// Headless provider park: the bridge answers the provider's own oneshot
    /// with DenyOnce immediately — the subprocess unparks denied — and files
    /// nothing.
    #[tokio::test]
    async fn bridge_headless_auto_denies_provider_park_without_filing() {
        let tmp = tempfile::tempdir().unwrap();
        let (agent, _session, pool) = park_test_agent(&tmp, GooseMode::Approve).await;
        agent.set_headless(true);

        let provider = Arc::new(ParkedProvider::new());
        let (tx, rx) = oneshot::channel();
        provider
            .pending
            .lock()
            .await
            .insert("req-headless-prov".to_string(), tx);
        *agent.provider.lock().await =
            Some(provider.clone() as Arc<dyn crate::providers::base::Provider>);

        let msg = provider_action_required("req-headless-prov", "Write");
        agent
            .bridge_provider_action_required(&msg, "sess-headless")
            .await;

        let confirmation = rx.await.expect("provider oneshot must receive the deny");
        assert_eq!(confirmation.permission, Permission::DenyOnce);
        assert!(
            !open_decision_exists(&pool, "req-headless-prov").await,
            "headless provider parks must not file"
        );
    }

    /// CLI-population provider park: the bridge must do NOTHING — no filing
    /// (undeliverable from another process) and no deny (the CLI terminal owns
    /// the answer; the park must stay live for it).
    #[tokio::test]
    async fn bridge_unregistered_process_leaves_park_untouched_and_files_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (agent, _session, pool) = park_test_agent(&tmp, GooseMode::Approve).await;

        let provider = Arc::new(ParkedProvider::new());
        let (tx, mut rx) = oneshot::channel();
        provider
            .pending
            .lock()
            .await
            .insert("req-cli-prov".to_string(), tx);
        *agent.provider.lock().await =
            Some(provider.clone() as Arc<dyn crate::providers::base::Provider>);

        let msg = provider_action_required("req-cli-prov", "Write");
        agent
            .bridge_provider_action_required(&msg, "sess-cli")
            .await;

        assert!(
            !open_decision_exists(&pool, "req-cli-prov").await,
            "unregistered process must not file for provider parks"
        );
        assert!(
            rx.try_recv().is_err(),
            "the park must stay live for the CLI terminal to answer"
        );
        assert!(
            provider.pending.lock().await.contains_key("req-cli-prov"),
            "pending confirmation must remain registered"
        );
    }

    /// Elicitation ActionRequired content is not a permission park — the
    /// bridge must ignore it entirely.
    #[tokio::test]
    async fn bridge_ignores_non_tool_confirmation_action_required() {
        let tmp = tempfile::tempdir().unwrap();
        let (agent, _session, pool) = park_test_agent(&tmp, GooseMode::Approve).await;
        agent.set_headless(true); // even the aggressive branch must not fire

        let mut msg = Message::assistant();
        msg.content
            .push(MessageContent::action_required_elicitation(
                "elic-1".to_string(),
                "need input".to_string(),
                serde_json::json!({"type": "object"}),
            ));
        agent
            .bridge_provider_action_required(&msg, "sess-elic")
            .await;

        assert!(!open_decision_exists(&pool, "elic-1").await);
    }
}
