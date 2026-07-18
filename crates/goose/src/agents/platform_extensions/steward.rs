//! Git Steward platform extension.
//!
//! Exposes two tools to the scheduled Steward recipe:
//!   - `propose_git_op` — the only in-process path to the safety core
//!     (`crate::steward`) for a *destructive* git operation. Enforces the
//!     protected-branch guard and risk classifier in code, then routes
//!     approved-for-review proposals to a human via the board.
//!   - `surface_steward_report` — surfaces the recipe's read/report items
//!     (stale merged branches, orphaned worktrees, repo-health flags) to BOTH
//!     a single board card (visual) and a recallable Brain memory
//!     (conversational), so the user can later ask what the Steward found.
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

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SurfaceReportParams {
    /// Absolute path of the repository the report covers.
    repo_path: String,
    /// One-line human summary, e.g. "12 stale merged branches, 6 orphaned worktrees".
    summary: String,
    /// Stale branches already merged to main (deletable). Each should ALSO be
    /// proposed via `propose_git_op` for its own approval card; list them here
    /// for the at-a-glance report.
    #[serde(default)]
    stale_merged_branches: Vec<String>,
    /// Worktrees whose branch is merged but were never reaped. Include the path
    /// and branch, e.g. "/Users/.../wt/foo (feature/foo, merged)".
    #[serde(default)]
    orphaned_worktrees: Vec<String>,
    /// Repo-health flags: branch counts, ahead/behind, uncommitted, etc.
    #[serde(default)]
    health_flags: Vec<String>,
    /// Project board to surface the report card on (ID or slug). Defaults to Personal.
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
                stale-branch reports, repo-health, CI investigation) does NOT use
                `propose_git_op` — do that directly with shell commands.

                When you have finished the read tasks, call `surface_steward_report` EXACTLY
                ONCE with your synthesized findings (stale merged branches, orphaned
                worktrees, repo-health flags). That posts a single summary card to the board
                and writes a recallable Brain memory so the user can later ask what you found.
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

    /// Surface a non-destructive repo-health report to BOTH the board (a single
    /// summary card) and the Brain (a clean, recallable memory). The Brain write
    /// is best-effort — a missing brain never fails the card.
    async fn handle_report(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: SurfaceReportParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("Invalid arguments: {e}"))?;

        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;

        let project_id = match params.project_id_or_slug.as_deref() {
            Some(id_or_slug) => Some(
                projects::get_project_by_id_or_slug(&pool, id_or_slug)
                    .await?
                    .ok_or_else(|| format!("Project '{}' not found", id_or_slug))?
                    .id,
            ),
            None => None,
        };

        let report = steward::RepoHealthReport {
            repo_path: params.repo_path.clone(),
            summary: params.summary.clone(),
            stale_merged_branches: params.stale_merged_branches.clone(),
            orphaned_worktrees: params.orphaned_worktrees.clone(),
            health_flags: params.health_flags.clone(),
            project_id,
        };

        // 1. Board card (visual, pull).
        let card_id = steward::surface_repo_health_report(&pool, report).await?;

        // 2. Recallable Brain memory (conversational, push) — a clean structured
        //    fact, not a raw transcript, written through the same global-brain
        //    seam the Librarian uses, so `recall "git steward"` returns it.
        let brain_memory_written = self.remember_report(&params).await;

        let json = serde_json::json!({
            "card_id": card_id,
            "summary": params.summary,
            "stale_merged_branches": params.stale_merged_branches.len(),
            "orphaned_worktrees": params.orphaned_worktrees.len(),
            "brain_memory_written": brain_memory_written,
        });
        Ok(vec![Content::text(format!(
            "Repo-health report surfaced to the board{}.\n\n{}",
            if brain_memory_written {
                " and recorded as a recallable Brain memory"
            } else {
                " (Brain unavailable — memory not written)"
            },
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ))])
    }

    /// Write the report as a clean, recallable Brain fact via the global-brain
    /// seam. Returns whether the write succeeded. Best-effort: a missing or
    /// failing brain is logged, not surfaced as a tool error.
    async fn remember_report(&self, p: &SurfaceReportParams) -> bool {
        let brain = match crate::agents::platform_extensions::get_global_brain() {
            Some(b) => b,
            None => return false,
        };

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let key = format!("steward-repo-health-{}", date);

        let mut content = format!(
            "Git Steward repo-health report ({date}) for {}: {}.",
            p.repo_path, p.summary
        );
        if !p.stale_merged_branches.is_empty() {
            content.push_str(&format!(
                " Stale merged branches ({}): {}.",
                p.stale_merged_branches.len(),
                p.stale_merged_branches.join(", ")
            ));
        }
        if !p.orphaned_worktrees.is_empty() {
            content.push_str(&format!(
                " Orphaned worktrees ({}): {}.",
                p.orphaned_worktrees.len(),
                p.orphaned_worktrees.join("; ")
            ));
        }
        if !p.health_flags.is_empty() {
            content.push_str(&format!(" Repo health: {}.", p.health_flags.join("; ")));
        }

        let device_id = *brain.device_id();
        match brain
            .remember_with(
                &key,
                &content,
                spectral::RememberOpts {
                    source: Some("steward".into()),
                    device_id: Some(device_id),
                    confidence: Some(1.0),
                    visibility: spectral::Visibility::Private,
                    wing: None,
                    ..Default::default()
                },
            )
            .await
        {
            Ok(_) => {
                tracing::info!(target: "steward", key = %key, "repo-health report remembered");
                true
            }
            Err(e) => {
                tracing::warn!(target: "steward", "failed to remember repo-health report: {e}");
                false
            }
        }
    }

    pub(crate) fn get_tools() -> Vec<Tool> {
        let schema = serde_json::to_value(schema_for!(ProposeGitOpParams)).unwrap();
        let report_schema = serde_json::to_value(schema_for!(SurfaceReportParams)).unwrap();
        vec![
            Tool::new(
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
            )),
            Tool::new(
                "surface_steward_report".to_string(),
                indoc! {r#"
                Surface your read/report repo-hygiene findings ONCE per run. Call this
                exactly once, after completing the read tasks, with the synthesized items:
                stale merged branches, orphaned worktrees, and repo-health flags.

                It creates a SINGLE summary card on the board AND writes a clean, recallable
                Brain memory, so the user can both see the items and later ask what the
                Steward found. This is informational — it changes nothing. Destructive
                cleanup (deleting a stale branch) still goes through `propose_git_op`
                separately, one approval card per branch.
            "#}
                .to_string(),
                report_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Surface Steward Report".to_string()),
                Some(false), // writes a card + memory
                Some(false), // not destructive — informational only
                Some(false),
                Some(false),
            )),
        ]
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
            "surface_steward_report" => self.handle_report(arguments).await,
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
