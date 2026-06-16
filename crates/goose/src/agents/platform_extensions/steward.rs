//! Git Steward platform extension.
//!
//! Exposes a single tool, `propose_git_op`, that the scheduled Steward recipe
//! calls for any *destructive* git operation it wants to perform. The tool is
//! the only in-process path to the safety core (`crate::steward`): it enforces
//! the protected-branch guard and the risk classifier in code, then routes
//! approved-for-review proposals to a human via the board.
//!
//! Disabled by default — only the Steward recipe enables it (`type: builtin,
//! name: steward`). Read/propose work uses the developer extension, not this.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::steward::{self, DestructiveProposal, GitOpKind, ProposalOutcome};
use crate::{cards, projects};
use anyhow::Result;
use async_trait::async_trait;
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool, ToolAnnotations,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "steward";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProposeGitOpParams {
    /// The destructive operation kind. One of: "branch_delete", "history_rewrite", "force_push".
    op_kind: String,
    /// The branch/ref the operation targets (e.g. "feature/stale-thing").
    branch: String,
    /// The exact git command you propose to run (e.g. "git branch -D feature/stale-thing").
    command: String,
    /// WHY this is proposed — must let a human judge from the card alone.
    /// Include the evidence: e.g. "stale 94 days, last commit abc1234, merged to main: yes".
    reason: String,
    /// Absolute path of the repository the operation applies to.
    repo_path: String,
    /// Project board to surface the approval card on (ID or slug). Defaults to Personal.
    project_id_or_slug: Option<String>,
}

pub struct StewardClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl StewardClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("Git Steward"),
            )
            .with_instructions(
                indoc! {r#"
                Git Steward safety gate. Use `propose_git_op` ONLY for destructive git
                operations: deleting a branch, rewriting history, or force-pushing.

                You must NEVER run a destructive git command yourself via the shell. Instead,
                describe the operation to `propose_git_op` with full reasoning. The tool will:
                  - HARD-REFUSE protected branches (main/master/develop/release-*/hotfix-*) —
                    no card is created and nothing happens; report the refusal.
                  - For all other branches, create an approval card on the board for the user
                    to review. The operation is NOT executed — it only becomes a proposal.

                Read/propose work (drafting commit messages, PR descriptions, changelogs,
                stale-branch reports, repo-health, CI investigation) does NOT use this tool —
                do that directly with shell commands and put the results in your report.
            "#}
                .to_string(),
            );
        Ok(Self { info, context })
    }

    async fn handle_propose(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let op_kind_str = args
            .get("op_kind")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: op_kind")?;
        let kind = GitOpKind::parse(op_kind_str).ok_or_else(|| {
            format!(
                "Invalid op_kind: '{}'. Must be one of: branch_delete, history_rewrite, force_push",
                op_kind_str
            )
        })?;
        if !kind.is_destructive() {
            return Err(format!(
                "op_kind '{}' is not destructive — handle it yourself, don't route it here.",
                op_kind_str
            ));
        }

        let branch = args
            .get("branch")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: branch")?
            .to_string();
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: command")?
            .to_string();
        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: reason")?
            .to_string();
        let repo_path = args
            .get("repo_path")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: repo_path")?
            .to_string();

        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;

        // Resolve the optional project to an ID; default (None) becomes Personal.
        let project_id = match args.get("project_id_or_slug").and_then(|v| v.as_str()) {
            Some(id_or_slug) => Some(
                projects::get_project_by_id_or_slug(&pool, id_or_slug)
                    .await?
                    .ok_or_else(|| format!("Project '{}' not found", id_or_slug))?
                    .id,
            ),
            None => None,
        };

        let proposal = DestructiveProposal {
            kind,
            branch: branch.clone(),
            command: command.clone(),
            reason,
            repo_path,
            project_id,
        };

        match steward::surface_destructive_proposal(&pool, proposal).await? {
            ProposalOutcome::HardBlocked { reason } => Ok(vec![Content::text(reason)]),
            ProposalOutcome::NotDestructive => Err(
                "Operation classified as non-destructive — no approval needed; handle it directly."
                    .to_string(),
            ),
            ProposalOutcome::CardCreated { card_id, risk } => {
                // Read back the card so the report shows what landed on the board.
                let card = cards::get_card(&pool, &card_id).await?;
                let json = serde_json::json!({
                    "card_id": card_id,
                    "risk_tier": format!("{:?}", risk),
                    "op_kind": kind.as_str(),
                    "branch": branch,
                    "command": command,
                    "title": card.as_ref().map(|c| c.title.clone()),
                });
                Ok(vec![Content::text(format!(
                    "Approval card created on the board (risk: {:?}). The operation was NOT executed — \
                     it awaits the user's review.\n\n{}",
                    risk,
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                ))])
            }
        }
    }

    fn get_tools() -> Vec<Tool> {
        let schema = serde_json::to_value(schema_for!(ProposeGitOpParams)).unwrap();
        vec![Tool::new(
            "propose_git_op".to_string(),
            indoc! {r#"
                Propose a DESTRUCTIVE git operation (branch_delete, history_rewrite, force_push)
                for human approval. NEVER run these commands yourself.

                Protected branches (main/master/develop/release-*/hotfix-*) are hard-refused:
                no card is created. For all other branches an approval card is placed on the
                board with your full reasoning, and the operation is NOT executed.

                Provide complete `reason` evidence (age, last commit sha, merge status) so the
                user can decide from the card alone.
            "#}
            .to_string(),
            schema.as_object().unwrap().clone(),
        )
        .annotate(ToolAnnotations::from_raw(
            Some("Propose Git Operation".to_string()),
            Some(false), // not read-only
            Some(false), // not destructive itself — it only proposes (creates a card)
            Some(false),
            Some(false),
        ))]
    }
}

#[async_trait]
impl McpClientTrait for StewardClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let content = match name {
            "propose_git_op" => self.handle_propose(arguments).await,
            _ => Err(format!("Unknown tool: {}", name)),
        };
        match content {
            Ok(content) => Ok(CallToolResult::success(content)),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {}",
                error
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}
