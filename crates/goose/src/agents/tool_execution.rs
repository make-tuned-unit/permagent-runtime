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
        }
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
use crate::conversation::message::{Message, ToolRequest};
use crate::session::Session;
use crate::tool_inspection::get_security_finding_id_from_results;

pub const DECLINED_RESPONSE: &str = "The user has declined to run this tool. \
    DO NOT attempt to call this tool again. \
    If there are no alternative methods to proceed, clearly explain the situation and STOP.";

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
                let security_message = inspection_results.iter()
                    .find(|result| result.tool_request_id == request.id)
                    .and_then(|result| {
                        if let crate::tool_inspection::InspectionAction::RequireApproval(Some(message)) = &result.action {
                            Some(message.clone())
                        } else {
                            None
                        }
                    });

                let confirmation_rx = self.tool_confirmation_router.register(request.id.clone()).await;

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
}
