//! Decision Inbox platform extension — the Inbox, reachable from chat.
//!
//! Two tools:
//!   - `list_pending_decisions` — read the open Decision Inbox so the agent
//!     can surface what is waiting, grouped into sensible bundles ("five
//!     stale goal-card reviews"), without the user navigating away.
//!   - `answer_decisions` — apply the user's stated approve/reject to one or
//!     more decisions. This acts for the USER, so it must only ever be called
//!     to carry out an instruction the user just gave in the conversation —
//!     never on the agent's own initiative. The tool-approval trust layer
//!     provides the real click for supervised modes.
//!
//! Effects: answering enqueues durable effects into the effect outbox, which
//! this module drains immediately after answering (the same worker path the
//! notification router uses). Kinds that need a live delivery channel
//! (tool approvals, session gates) are refused here and stay Inbox-only.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::decisions;
use anyhow::Result;
use async_trait::async_trait;
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "inbox";

/// Audit principal recorded when an answer arrives through chat, so the
/// append-only audit can distinguish a chat-relayed answer from the Inbox UI.
const CHAT_PRINCIPAL: &str = "henry-chat";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct AnswerDecisionsParams {
    /// The decision ids to answer, exactly as returned by
    /// `list_pending_decisions`. Bundle related decisions the user approved
    /// together into ONE call.
    decision_ids: Vec<String>,
    /// "approve" or "reject" — the user's stated verdict, applied to every id.
    answer: String,
    /// Optional note recorded on each decision (e.g. the user's reason).
    note: Option<String>,
}

pub struct InboxClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl InboxClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("Decision Inbox"),
            )
            .with_instructions(
                indoc! {r#"
                The user's Decision Inbox, from chat.

                `list_pending_decisions` any time the user asks what needs them, or
                when surfacing approvals would save them a trip to the Inbox. GROUP
                related items into bundles ("5 approve_review decisions on goal
                cards we already finished") and offer them as one decision.

                `answer_decisions` ONLY to carry out a verdict the user just gave in
                this conversation. Never answer on your own judgment, never infer
                approval from silence, and read back what you are about to apply
                ("approving these 5: …") before calling. Kinds that need a live
                channel (tool approvals, session gates) are refused here — send the
                user to the Inbox for those.
            "#}
                .to_string(),
            );
        Ok(Self { info, context })
    }

    async fn handle_list(&self) -> Result<Vec<Content>, String> {
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let items = decisions::list_open_decisions(&pool).await?;
        if items.is_empty() {
            return Ok(vec![Content::text(
                "The Decision Inbox is empty — nothing is waiting on the user.",
            )]);
        }
        let mut lines = vec![format!(
            "{} open decision{} (newest first). Bundle related ones when offering them:",
            items.len(),
            if items.len() == 1 { "" } else { "s" }
        )];
        for item in &items {
            let d = &item.decision;
            let bulkable = decisions::effect_outbox_claim_key(&d.id, &d.kind).is_some();
            lines.push(format!(
                "- id: {} · kind: {} · tier {} · {}{}{}\n  {}",
                d.id,
                d.kind,
                d.tier,
                d.headline,
                item.goal_title
                    .as_deref()
                    .map(|g| format!(" · goal: {g}"))
                    .unwrap_or_default(),
                if bulkable {
                    ""
                } else {
                    " · [Inbox-only — cannot be answered from chat]"
                },
                d.detail.chars().take(200).collect::<String>(),
            ));
        }
        Ok(vec![Content::text(lines.join("\n"))])
    }

    async fn handle_answer(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: AnswerDecisionsParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("Invalid parameters: {e}"))?;
        if params.decision_ids.is_empty() {
            return Err("decision_ids is empty — list first, then answer.".to_string());
        }
        if params.answer != "approve" && params.answer != "reject" {
            return Err(format!(
                "answer must be \"approve\" or \"reject\", got \"{}\". Choice/input/edit \
                 decisions are answered from the Inbox.",
                params.answer
            ));
        }

        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;

        let mut applied = Vec::new();
        let mut refused = Vec::new();
        for id in &params.decision_ids {
            // Live-channel kinds are refused BEFORE answering: their effect
            // cannot ride the outbox, so answering here would strand them.
            let kind = match decisions::get_decision(&pool, id).await? {
                Some(d) => d.kind,
                None => {
                    refused.push(format!("{id}: not found"));
                    continue;
                }
            };
            if decisions::effect_outbox_claim_key(id, &kind).is_none() {
                refused.push(format!(
                    "{id}: kind '{kind}' needs a live channel — answer it from the Inbox"
                ));
                continue;
            }
            let answer = decisions::DecisionAnswer {
                answer: params.answer.clone(),
                note: params.note.clone(),
                choice_id: None,
                input_text: None,
            };
            match decisions::answer_decision_with_principal(
                &pool,
                id,
                &answer,
                decisions::ACTOR_JESSE,
                CHAT_PRINCIPAL,
            )
            .await
            {
                Ok((d, _proof)) => {
                    applied.push(format!("{id}: {} — {}", params.answer, d.headline))
                }
                Err(e) => refused.push(format!("{id}: {e}")),
            }
        }

        // Apply the enqueued effects now rather than waiting for the router's
        // next drain tick — the user is watching.
        if !applied.is_empty() {
            if let Err(e) = crate::decisions_effects::drain_effect_outbox(&pool).await {
                refused.push(format!("effect drain reported: {e}"));
            }
        }

        let mut out = format!(
            "{} of {} decision{} {}d.",
            applied.len(),
            params.decision_ids.len(),
            if params.decision_ids.len() == 1 {
                ""
            } else {
                "s"
            },
            params.answer
        );
        if !applied.is_empty() {
            out.push_str(&format!("\n\nApplied:\n{}", applied.join("\n")));
        }
        if !refused.is_empty() {
            out.push_str(&format!("\n\nNot applied:\n{}", refused.join("\n")));
        }
        Ok(vec![Content::text(out)])
    }
}

impl InboxClient {
    pub(crate) fn get_tools() -> Vec<Tool> {
        let empty: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object", "properties": {}, "required": []
        }))
        .expect("static schema");
        let answer_schema: JsonObject = serde_json::to_value(schema_for!(AnswerDecisionsParams))
            .expect("static schema")
            .as_object()
            .expect("schema is an object")
            .clone();

        vec![
            Tool::new(
                "list_pending_decisions".to_string(),
                "Read the user's open Decision Inbox — everything waiting on their approval \
                 (goal reviews, unblocks, proposals). Use when they ask what needs them, or to \
                 surface approvals right here in chat instead of sending them to the Inbox. \
                 Group related items into bundles they can approve in one breath."
                    .to_string(),
                empty,
            ),
            Tool::new(
                "answer_decisions".to_string(),
                "Apply the user's explicit approve/reject — stated in THIS conversation — to one \
                 or more decisions by id. Bundle what they approved together into one call, and \
                 read the list back to them first. NEVER call this on your own judgment; the \
                 verdict must be the user's words."
                    .to_string(),
                answer_schema,
            ),
        ]
    }
}

#[async_trait]
impl McpClientTrait for InboxClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }

    async fn call_tool(
        &self,
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let result = match name {
            "list_pending_decisions" => self.handle_list().await,
            "answer_decisions" => self.handle_answer(arguments).await,
            _ => Err(format!("Unknown tool: {name}")),
        };
        match result {
            Ok(content) => Ok(CallToolResult::success(content)),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
        }
    }
}
