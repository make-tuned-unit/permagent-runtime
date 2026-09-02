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
use crate::session::SessionType;
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

/// The highest tier answerable from chat. Tier 2 is the human-only tier —
/// irreversible or high-blast-radius actions (goal deletion, merge/push to
/// main, secrets access, spend, policy edits). Those are gated in code on
/// `acted_by == ACTOR_JESSE`, and a tool call is the model's word, not the
/// user's hand: relaying one as the user would let model output satisfy the exact
/// checkpoint that exists to require a human. Chat answers act as Henry and
/// stop here.
const MAX_TIER_FROM_CHAT: i64 = 1;

/// The name of the write tool, so the capability filter and the dispatch
/// refusal cannot drift apart.
const ANSWER_TOOL: &str = "answer_decisions";

/// Whether a session holds the decision-answering capability (D30).
///
/// The tier ceiling above answers "which decisions may an actor settle"; this
/// answers the question nothing used to ask — "may this CALLER act as that
/// actor at all". Answering a decision is the user's verdict relayed by the
/// model; a session with no human reading the turn has no verdict to relay, so
/// the tool is not a tool it may be told not to call — it is a capability it
/// does not hold. Absent from `list_tools`, refused at dispatch.
///
/// Precedent for the enforcement point: `summon.rs` filters `delegate` out of
/// a `SessionType::SubAgent` session's tool list AND refuses it in the handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerCapability {
    /// A human is watching this surface and can state the verdict being relayed.
    Granted,
    /// Structurally withheld from this session type — nobody is watching.
    Withheld(SessionType),
    /// The session could not be resolved, so nothing vouches for a human being
    /// present. Fail-safe defaults: no evidence of a human ⇒ no authority.
    WithheldUnknownSession,
}

impl AnswerCapability {
    pub fn for_session_type(session_type: SessionType) -> Self {
        if session_type.is_interactive() {
            Self::Granted
        } else {
            Self::Withheld(session_type)
        }
    }

    pub fn is_granted(&self) -> bool {
        matches!(self, Self::Granted)
    }

    /// The refusal text handed back to the model. Names the reason, so the
    /// model reports the boundary instead of retrying against it.
    pub fn refusal(&self) -> Option<String> {
        match self {
            Self::Granted => None,
            Self::Withheld(session_type) => Some(format!(
                "answer_decisions is not available to a '{session_type}' session. Answering a \
                 decision relays a verdict the user just gave out loud, and no user is watching \
                 this session — a goal worker, a delegated subagent and a scheduled run all \
                 answer to nobody. Leave the decision open: it is waiting for the user in the \
                 Decision Inbox, and approving your own work would defeat the review that \
                 decision exists to be."
            )),
            Self::WithheldUnknownSession => Some(
                "answer_decisions is unavailable: this session could not be identified, so \
                 nothing establishes that a user is present to give the verdict. The decision \
                 stays open in the Decision Inbox."
                    .to_string(),
            ),
        }
    }
}

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
                ("approving these 5: …") before calling.

                Prefer settling decisions HERE. Do not send the user to the Decision
                Inbox when they can say yes/no in this conversation or tap Approve
                on the card in chat. Kinds that need a live channel (tool approvals,
                session gates) cannot be answered by this tool — those show Approve
                buttons in chat and on the voice orb; tell the user to tap or say
                yes there, not to leave the conversation.
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
                    " · [tap Approve in this chat or say yes — cannot be answered by this tool]"
                },
                d.detail.chars().take(200).collect::<String>(),
            ));
        }
        Ok(vec![Content::text(lines.join("\n"))])
    }

    /// Resolve the answering session's capability. Fails CLOSED: an
    /// unresolvable session is treated as having no human behind it.
    async fn answer_capability(&self, session_id: &str) -> AnswerCapability {
        match self
            .context
            .session_manager
            .get_session(session_id, false)
            .await
        {
            Ok(session) => AnswerCapability::for_session_type(session.session_type),
            Err(e) => {
                tracing::warn!(
                    session_id,
                    error = %e,
                    "could not resolve session type for the decision-answering capability; \
                     withholding answer_decisions"
                );
                AnswerCapability::WithheldUnknownSession
            }
        }
    }

    async fn handle_answer(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        // Capability gate FIRST — before parsing, before touching the pool.
        // A session that does not hold the capability never reaches a write.
        if let Some(refusal) = self.answer_capability(session_id).await.refusal() {
            tracing::warn!(
                session_id,
                "refused answer_decisions: session type does not hold the capability"
            );
            return Err(refusal);
        }

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
            let (kind, tier) = match decisions::get_decision(&pool, id).await? {
                Some(d) => (d.kind, d.tier),
                None => {
                    refused.push(format!("{id}: not found"));
                    continue;
                }
            };
            // Refuse the human-only tier explicitly, so the reason is legible
            // rather than surfacing as an opaque Forbidden from the tier gate.
            if tier > MAX_TIER_FROM_CHAT {
                refused.push(format!(
                    "{id}: tier {tier} needs your own hand — tap Approve in this chat or say yes on voice"
                ));
                continue;
            }
            if decisions::effect_outbox_claim_key(id, &kind).is_none() {
                refused.push(format!(
                    "{id}: kind '{kind}' needs a live channel — tap Approve in this chat or say yes on voice"
                ));
                continue;
            }
            let answer = decisions::DecisionAnswer {
                answer: params.answer.clone(),
                note: params.note.clone(),
                choice_id: None,
                input_text: None,
            };
            match decisions::answer_decision_as_session(
                &pool,
                id,
                &answer,
                // NEVER ACTOR_JESSE: see MAX_TIER_FROM_CHAT.
                decisions::ACTOR_HENRY,
                CHAT_PRINCIPAL,
                // Declaring the session arms the self-reference block: even a
                // capability-holding chat session may not settle a decision
                // that judges its own work.
                session_id,
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
                 surface approvals right here in chat instead of sending them away. \
                 Group related items into bundles they can approve in one breath. \
                 Never tell them to open the Inbox when they can approve here."
                    .to_string(),
                empty,
            ),
            Tool::new(
                ANSWER_TOOL.to_string(),
                "Apply the user's explicit approve/reject — stated in THIS conversation — to one \
                 or more decisions by id. Bundle what they approved together into one call, and \
                 read the list back to them first. NEVER call this on your own judgment; the \
                 verdict must be the user's words. High-tier decisions (deletions, merges, \
                 spend, secrets) and live-channel kinds (tool approvals, session gates) are \
                 refused here — tell them to tap Approve in this chat (or say yes on voice), \
                 not to leave for the Inbox."
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
        session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        // Select from the full inventory so `get_tools()` stays the single
        // declared superset (self-knowledge's drift guard reads it), and drop
        // what this session type does not hold — mirroring summon's filter.
        let mut tools = Self::get_tools();
        if !self.answer_capability(session_id).await.is_granted() {
            // Reading the inbox stays available to every session: a worker may
            // legitimately need to know what is waiting. Only the write goes.
            tools.retain(|t| t.name.as_ref() != ANSWER_TOOL);
        }
        Ok(ListToolsResult {
            tools,
            next_cursor: None,
            meta: None,
        })
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }

    async fn call_tool(
        &self,
        ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let result = match name {
            "list_pending_decisions" => self.handle_list().await,
            ANSWER_TOOL => self.handle_answer(&ctx.session_id, arguments).await,
            _ => Err(format!("Unknown tool: {name}")),
        };
        match result {
            Ok(content) => Ok(CallToolResult::success(content)),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D30: the capability is an ALLOW list over session types. Only the two
    /// surfaces with a human reading the turn hold it.
    #[test]
    fn only_interactive_session_types_hold_the_answer_capability() {
        for granted in [SessionType::User, SessionType::Terminal] {
            assert_eq!(
                AnswerCapability::for_session_type(granted),
                AnswerCapability::Granted,
                "{granted} is an interactive surface — chat must keep answering"
            );
        }
        for withheld in [
            SessionType::SubAgent,  // goal workers AND summoned children
            SessionType::Scheduled, // cron recipes: nobody is watching
            SessionType::Hidden,
            SessionType::Gateway,
            SessionType::Acp,
        ] {
            assert_eq!(
                AnswerCapability::for_session_type(withheld),
                AnswerCapability::Withheld(withheld),
                "{withheld} runs unattended and must not answer decisions"
            );
        }
    }

    #[test]
    fn a_withheld_capability_returns_a_typed_refusal_naming_the_session_type() {
        let refusal = AnswerCapability::for_session_type(SessionType::SubAgent)
            .refusal()
            .expect("a withheld capability must explain itself");
        assert!(refusal.contains("sub_agent"), "got: {refusal}");
        assert!(
            refusal.contains("Decision Inbox"),
            "the refusal must say where the decision goes instead: {refusal}"
        );
        assert!(AnswerCapability::for_session_type(SessionType::Scheduled)
            .refusal()
            .unwrap()
            .contains("scheduled"));
    }

    #[test]
    fn an_unresolvable_session_fails_closed() {
        let cap = AnswerCapability::WithheldUnknownSession;
        assert!(!cap.is_granted());
        assert!(cap.refusal().is_some());
    }

    #[test]
    fn a_granted_capability_refuses_nothing() {
        let cap = AnswerCapability::for_session_type(SessionType::User);
        assert!(cap.is_granted());
        assert!(cap.refusal().is_none());
    }

    /// The declared superset stays whole — self-knowledge's drift guard reads
    /// `get_tools()`, and `list_tools` narrows from it per session.
    #[test]
    fn the_declared_inventory_still_carries_both_tools() {
        let names: Vec<String> = InboxClient::get_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(names.iter().any(|n| n == "list_pending_decisions"));
        assert!(names.iter().any(|n| n == ANSWER_TOOL));
    }

    /// The filter `list_tools` applies, exercised without a live session row:
    /// a withheld capability drops exactly the write tool and keeps the read.
    #[test]
    fn the_capability_filter_drops_only_the_write_tool() {
        let mut tools = InboxClient::get_tools();
        let cap = AnswerCapability::for_session_type(SessionType::SubAgent);
        if !cap.is_granted() {
            tools.retain(|t| t.name.as_ref() != ANSWER_TOOL);
        }
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name.as_ref(), "list_pending_decisions");
    }
}
