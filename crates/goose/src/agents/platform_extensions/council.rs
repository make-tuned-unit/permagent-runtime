//! The Council — Henry's tools to convene a debate and read the last report.
//!
//! Extension registry key is `deliberate` so it does not collide with the
//! worker id `council` in the self-knowledge namespace (Forecaster uses the
//! same split: extension `forecast`, character `forecaster`).

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::council::{self, debate, store};
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "deliberate";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ConveneParams {
    /// Optional extra question to put in front of the council, on top of the
    /// portfolio brief (e.g. "are we over-rotated on Permagent?").
    question: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ReportParams {
    /// Session id from a previous council_convene. Omit for the latest report.
    session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct StatusParams {}

pub struct CouncilClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl CouncilClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title(council::AGENT_NAME),
            )
            .with_instructions(
                "The Council briefs every connected chat-completion provider on the \
                 current state of the work, they debate, and you chair a weekly report. \
                 council_status is the cheap live query (on/off, seats, last headline, \
                 open inbox actions) — use it when asked how the Council is doing. \
                 council_report reads the latest (or a named) report including per-model \
                 dissent. council_convene runs a session (it spends every connected \
                 provider) — never convene just to check status. Actions land in the \
                 Decision Inbox as proposals — you do not impersonate the other models \
                 and you do not act on the report yourself.",
            );
        Ok(Self { info, context })
    }

    async fn pool(&self) -> std::result::Result<sqlx::Pool<sqlx::Sqlite>, String> {
        self.context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())
    }

    pub(crate) fn get_tools() -> Vec<Tool> {
        let convene_schema = serde_json::to_value(schema_for!(ConveneParams))
            .unwrap()
            .as_object()
            .unwrap()
            .clone();
        let report_schema = serde_json::to_value(schema_for!(ReportParams))
            .unwrap()
            .as_object()
            .unwrap()
            .clone();
        let status_schema = serde_json::to_value(schema_for!(StatusParams))
            .unwrap()
            .as_object()
            .unwrap()
            .clone();
        vec![
            Tool::new(
                "council_status".to_string(),
                "Live query: whether The Council is on, which chat-completion providers sit, the last headline, and how many Decision Inbox actions are still open. Cheap — does not run a debate. Works while the flag is off. Use this when asked how the Council is doing; do not council_convene just to check.".to_string(),
                status_schema,
            ),
            Tool::new(
                "council_convene".to_string(),
                "Brief every connected chat provider on the current state of the work, run a two-round debate, and chair a weekly report. Optional question is added to the brief. Spends API credits on every seated model. Actions land as Decision Inbox proposals.".to_string(),
                convene_schema,
            ),
            Tool::new(
                "council_report".to_string(),
                "Read the latest Council weekly report (or a named session), including per-model dissent.".to_string(),
                report_schema,
            ),
        ]
    }

    async fn handle_status(&self) -> std::result::Result<CallToolResult, String> {
        let pool = self.pool().await.ok();
        let text = council::format_status(pool.as_ref()).await;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    async fn handle_convene(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        if !council::is_enabled() {
            return Ok(CallToolResult::error(vec![Content::text(
                "The Council is off. Turn on council_enabled under Settings → Features, then try again."
                    .to_string(),
            )]));
        }
        let params: ConveneParams =
            serde_json::from_value(serde_json::Value::Object(arguments.unwrap_or_default()))
                .unwrap_or(ConveneParams { question: None });
        let pool = self.pool().await?;
        let question = params
            .question
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        match council::convene(
            &pool,
            store::Trigger::OnDemand,
            question,
            &debate::LiveCaller,
        )
        .await
        {
            Ok(c) => {
                let body = format!(
                    "Council session {} ({:?}). {} of {} members answered. Headline: {}\n\n{}\n\n{} action(s) filed in the Decision Inbox.",
                    c.session_id,
                    c.status,
                    c.n_ok,
                    c.n_members,
                    c.headline,
                    c.markdown,
                    c.n_actions
                );
                Ok(CallToolResult::success(vec![Content::text(body)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
        }
    }

    async fn handle_report(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let params: ReportParams =
            serde_json::from_value(serde_json::Value::Object(arguments.unwrap_or_default()))
                .unwrap_or(ReportParams { session_id: None });
        let pool = self.pool().await?;
        match council::format_report(&pool, params.session_id.as_deref()).await {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
        }
    }
}

#[async_trait]
impl McpClientTrait for CouncilClient {
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
        match name {
            "council_status" => match self.handle_status().await {
                Ok(r) => Ok(r),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
            },
            "council_convene" => match self.handle_convene(arguments).await {
                Ok(r) => Ok(r),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
            },
            "council_report" => match self.handle_report(arguments).await {
                Ok(r) => Ok(r),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
            },
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown tool: {name}"
            ))])),
        }
    }
}
