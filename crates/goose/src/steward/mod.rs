//! Git Steward — safety core for autonomous repo hygiene.
//!
//! The Steward runs as a scheduled, local-model-driven recipe (see
//! `crates/goose-server/src/automation/steward.yaml`). Read/propose work
//! (commit messages, PR descriptions, changelogs, stale-branch reports,
//! repo-health, CI-failure investigation) is done autonomously by the recipe
//! via the developer (shell) extension and never touches this module.
//!
//! This module is the *destructive* path. It enforces two things that must
//! NOT live in a prompt (a local 14B can be cajoled; code cannot):
//!   1. A protected-branch guard that hard-refuses delete/rewrite/force-push on
//!      `main`/`master`/`develop`/`release-*`/`hotfix-*` — independent of any
//!      approval surface. No card is ever created for these.
//!   2. A git-op → [`RiskLevel`] classifier that decides whether an op runs
//!      autonomously or must be routed to a human for approval.
//!
//! Destructive proposals that clear the guard are surfaced to a human via
//! [`surface_destructive_proposal`] — the single abstracted routing seam.

pub mod secret_scan;

use crate::cards::{self, CreateCard};
use crate::security::patterns::RiskLevel;
use sqlx::{Pool, Sqlite};

/// The kinds of git operations the Steward reasons about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOpKind {
    /// Read-only inspection (status, log, diff, branch list). Always autonomous.
    Read,
    /// Create a new branch. Benign, recoverable.
    BranchCreate,
    /// A local commit (no push). Recoverable from reflog.
    LocalCommit,
    /// Delete a branch. Destructive — requires approval.
    BranchDelete,
    /// Rewrite history (rebase, reset --hard, filter-branch). Destructive.
    HistoryRewrite,
    /// Force-push to a remote. Destructive and remote-visible.
    ForcePush,
}

impl GitOpKind {
    /// Parse the wire string used by the `propose_git_op` tool.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Self::Read),
            "branch_create" => Some(Self::BranchCreate),
            "local_commit" => Some(Self::LocalCommit),
            "branch_delete" => Some(Self::BranchDelete),
            "history_rewrite" => Some(Self::HistoryRewrite),
            "force_push" => Some(Self::ForcePush),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::BranchCreate => "branch_create",
            Self::LocalCommit => "local_commit",
            Self::BranchDelete => "branch_delete",
            Self::HistoryRewrite => "history_rewrite",
            Self::ForcePush => "force_push",
        }
    }

    /// Destructive ops are the ones that can lose work or rewrite shared state.
    pub fn is_destructive(self) -> bool {
        matches!(
            self,
            Self::BranchDelete | Self::HistoryRewrite | Self::ForcePush
        )
    }
}

/// What the Steward is allowed to do with a given op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routing {
    /// Steward may act / report without asking.
    Autonomous,
    /// Must be routed to a human (a board card today; the decision-inbox later).
    RequiresApproval,
    /// Refused unconditionally — protected branch. No card, ever.
    HardBlocked,
}

/// Branch names the Steward must never delete, rewrite, or force-push.
///
/// Exact matches plus the `release-*` / `hotfix-*` prefixes from the dispatch.
/// The slash variants (`release/`, `hotfix/`) are included as a stricter
/// superset — over-blocking is the safe direction here.
const PROTECTED_EXACT: &[&str] = &["main", "master", "develop"];
const PROTECTED_PREFIXES: &[&str] = &["release-", "release/", "hotfix-", "hotfix/"];

/// Normalize a ref to a bare branch name for guard matching.
/// Strips `refs/heads/` and a single leading `<remote>/` segment.
fn normalize_branch(branch: &str) -> &str {
    let b = branch.trim();
    let b = b.strip_prefix("refs/heads/").unwrap_or(b);
    // Strip a leading remote segment (e.g. "origin/main" -> "main"), but only
    // for the common single-segment remote case; protected names have no slash
    // and protected prefixes are matched separately below.
    if let Some((first, rest)) = b.split_once('/') {
        if !rest.is_empty()
            && !PROTECTED_PREFIXES.iter().any(|p| b.starts_with(p))
            && first != "refs"
        {
            // e.g. "origin/main" -> "main"; leave "release/1.2" intact (prefix-matched).
            return rest.split('/').next_back().unwrap_or(rest);
        }
    }
    b
}

/// True if this branch is hardcoded never-touch for destructive ops.
pub fn is_protected_branch(branch: &str) -> bool {
    let raw = branch.trim();
    let name = normalize_branch(raw);
    // Check both the raw ref and the normalized name against the prefixes, so
    // "origin/release-1.0" and "release-1.0" are both caught.
    PROTECTED_EXACT.contains(&name)
        || PROTECTED_PREFIXES
            .iter()
            .any(|p| name.starts_with(p) || raw.starts_with(p))
}

/// Map a git op to its security risk level.
pub fn classify_risk(kind: GitOpKind) -> RiskLevel {
    match kind {
        GitOpKind::Read | GitOpKind::BranchCreate => RiskLevel::Low,
        GitOpKind::LocalCommit => RiskLevel::Medium,
        GitOpKind::BranchDelete => RiskLevel::High,
        GitOpKind::HistoryRewrite | GitOpKind::ForcePush => RiskLevel::Critical,
    }
}

/// Decide how a proposed op should be handled.
///
/// Protected-branch destructive ops are [`Routing::HardBlocked`] *before* any
/// risk consideration — the guard is independent of the approval surface.
pub fn route(kind: GitOpKind, branch: &str) -> Routing {
    if kind.is_destructive() && is_protected_branch(branch) {
        return Routing::HardBlocked;
    }
    match classify_risk(kind) {
        RiskLevel::High | RiskLevel::Critical => Routing::RequiresApproval,
        RiskLevel::Low | RiskLevel::Medium => Routing::Autonomous,
    }
}

/// A destructive op the Steward wants a human to approve.
#[derive(Debug, Clone)]
pub struct DestructiveProposal {
    pub kind: GitOpKind,
    /// The branch/ref the op targets.
    pub branch: String,
    /// The exact git command the Steward proposes to run.
    pub command: String,
    /// Why the Steward proposes it — must let a human judge from the card alone
    /// (e.g. "stale 94 days, last commit abc1234, merged to main: yes").
    pub reason: String,
    /// Absolute path of the repo the op applies to.
    pub repo_path: String,
    /// Project board to surface the card on. Defaults to Personal if None.
    pub project_id: Option<String>,
}

/// Outcome of surfacing a proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalOutcome {
    /// A board card was created for human approval.
    CardCreated { card_id: String, risk: RiskLevel },
    /// Refused — protected branch. No card created.
    HardBlocked { reason: String },
    /// Op was not destructive; the Steward should handle it autonomously, not
    /// route it here. No card created.
    NotDestructive,
}

/// Surface a destructive proposal to a human — the single abstracted routing seam.
///
/// **v1 (today):** creates a card on the project's Kanban board via
/// [`cards::create_card`] (`created_by = "henry"`), because the board is the
/// only *live* human-approval surface on `main`. The richer decision-inbox
/// (PR #314: `crate::decision_inbox` / `crate::decisions`) is not in `main` yet
/// and stays dormant behind `orchestrator.enabled` even once merged — routing
/// there now would put approval items on a surface the user can't see.
///
/// **Swap (later):** once #314 is merged AND orchestrator is enabled, replace
/// the `cards::create_card(...)` call below with the decisions API
/// (`decisions::create_decision(...)`). That is the only line that changes; the
/// guard, classifier, and call sites stay identical.
pub async fn surface_destructive_proposal(
    pool: &Pool<Sqlite>,
    proposal: DestructiveProposal,
) -> Result<ProposalOutcome, String> {
    match route(proposal.kind, &proposal.branch) {
        Routing::HardBlocked => {
            let reason = format!(
                "REFUSED: '{}' is a protected branch — {} is never permitted by the Steward. No approval card created.",
                proposal.branch,
                proposal.kind.as_str()
            );
            // Guard refusal is logged independently of any approval surface.
            tracing::warn!(
                target: "steward",
                branch = %proposal.branch,
                op = %proposal.kind.as_str(),
                command = %proposal.command,
                "protected-branch guard refused destructive git op"
            );

            // A guard refusal deliberately creates NO card — there is nothing
            // for a human to approve. But it must not vanish into a log either:
            // the Steward attempting a protected-branch op is exactly the kind
            // of thing Henry should be able to raise. Reported, not actioned.
            crate::briefings::file_briefing(
                pool,
                crate::briefings::NewBriefing {
                    from_agent: "steward".to_string(),
                    kind: "guard_refusal".to_string(),
                    severity: crate::briefings::Severity::Attention,
                    summary: format!(
                        "Guard refused {} on protected branch `{}` ({}). No card created.",
                        proposal.kind.as_str(),
                        proposal.branch,
                        proposal.repo_path,
                    ),
                    detail: Some(format!(
                        "Attempted command: {}\nStated reason: {}",
                        proposal.command, proposal.reason
                    )),
                    ref_kind: None,
                    ref_id: None,
                },
            )
            .await;

            Ok(ProposalOutcome::HardBlocked { reason })
        }
        Routing::Autonomous => Ok(ProposalOutcome::NotDestructive),
        Routing::RequiresApproval => {
            let risk = classify_risk(proposal.kind);
            let project_id = proposal
                .project_id
                .clone()
                .unwrap_or_else(|| crate::projects::PERSONAL_PROJECT_ID.to_string());

            let title = format!(
                "Steward: approve {} on `{}`",
                proposal.kind.as_str(),
                proposal.branch
            );
            let description = format!(
                "The Git Steward proposes a destructive operation that needs your approval.\n\n\
                 **Operation:** {}\n\
                 **Branch:** {}\n\
                 **Risk tier:** {:?}\n\
                 **Repo:** {}\n\n\
                 **Proposed command:**\n```\n{}\n```\n\n\
                 **Why:** {}\n\n\
                 _Approve only if you've confirmed the reasoning above. The Steward never \
                 executes this itself in v1 — it only proposes._",
                proposal.kind.as_str(),
                proposal.branch,
                risk,
                proposal.repo_path,
                proposal.command,
                proposal.reason,
            );

            // Everything a human needs to judge — and the machine-readable shape
            // the future decision-inbox swap will consume.
            let metadata_json = serde_json::json!({
                "steward": true,
                "op_kind": proposal.kind.as_str(),
                "branch": proposal.branch,
                "command": proposal.command,
                "risk_tier": format!("{:?}", risk),
                "reason": proposal.reason,
                "repo_path": proposal.repo_path,
                "needs_human_attention": true,
            });

            let card = cards::create_card(
                pool,
                CreateCard {
                    project_id,
                    title,
                    description: Some(description),
                    card_type: Some("standard".to_string()),
                    column_id: None,
                    // Attribution note: this card is the STEWARD's, not Henry's,
                    // and `metadata_json.steward = true` above records that. It
                    // is still authored as "henry" because `cards.created_by`
                    // is constrained by a DB CHECK that does not include
                    // "steward"; widening it needs an in-place table rebuild.
                    // Changing this string without that migration makes card
                    // creation FAIL, which would silently drop destructive-op
                    // proposals that must reach a human — the exact outcome the
                    // Steward exists to prevent.
                    created_by: Some("henry".to_string()),
                    metadata_json: Some(metadata_json),
                },
            )
            .await;

            // If the board write fails, the proposal has nowhere to land: this
            // runs inside a scheduled job, so the propagated error becomes a log
            // line nobody is watching, and a destructive operation the Steward
            // wanted a human to rule on just disappears. Report the failure
            // itself so Henry can say "I tried to raise something and could
            // not", then still propagate.
            let card = match card {
                Ok(card) => card,
                Err(e) => {
                    crate::briefings::file_briefing(
                        pool,
                        crate::briefings::NewBriefing {
                            from_agent: "steward".to_string(),
                            kind: "proposal_delivery_failed".to_string(),
                            severity: crate::briefings::Severity::ActionRequired,
                            summary: format!(
                                "Could NOT record a proposed {} on `{}` ({}) — the approval card \
                                 failed to save, so it is not on the board.",
                                proposal.kind.as_str(),
                                proposal.branch,
                                proposal.repo_path,
                            ),
                            detail: Some(format!(
                                "Card write error: {e}\nProposed command: {}\nReason: {}",
                                proposal.command, proposal.reason
                            )),
                            ref_kind: None,
                            ref_id: None,
                        },
                    )
                    .await;
                    return Err(e);
                }
            };

            tracing::info!(
                target: "steward",
                card_id = %card.id,
                branch = %proposal.branch,
                op = %proposal.kind.as_str(),
                risk = ?risk,
                "destructive git op routed to board for approval"
            );

            // Report up to Henry. The card is where the human decides; this is
            // so Henry knows a decision is pending and can raise it unprompted
            // instead of the board waiting to be noticed.
            crate::briefings::file_briefing(
                pool,
                crate::briefings::NewBriefing {
                    from_agent: "steward".to_string(),
                    kind: "destructive_proposal".to_string(),
                    severity: crate::briefings::Severity::ActionRequired,
                    summary: format!(
                        "Proposed {} on `{}` ({}) — awaiting approval on the board.",
                        proposal.kind.as_str(),
                        proposal.branch,
                        proposal.repo_path,
                    ),
                    detail: Some(proposal.reason.clone()),
                    ref_kind: Some("card".to_string()),
                    ref_id: Some(card.id.clone()),
                },
            )
            .await;

            Ok(ProposalOutcome::CardCreated {
                card_id: card.id,
                risk,
            })
        }
    }
}

/// A non-destructive repo-hygiene summary the Steward surfaces for awareness.
///
/// Unlike [`DestructiveProposal`], this proposes nothing — it informs. The
/// read/report items (stale merged branches, orphaned worktrees, repo-health
/// flags) land on the board as a single summary card; the destructive cleanup
/// for each stale branch still goes through [`surface_destructive_proposal`] as
/// its own approval card.
#[derive(Debug, Clone)]
pub struct RepoHealthReport {
    /// Absolute path of the repo the report covers.
    pub repo_path: String,
    /// One-line human summary (e.g. "12 stale merged branches, 6 orphaned worktrees").
    pub summary: String,
    /// Stale branches already merged to main — safe to delete. Each also gets
    /// its own approval card via `propose_git_op`; listed here for the
    /// at-a-glance view.
    pub stale_merged_branches: Vec<String>,
    /// Worktrees whose branch is merged but the worktree was never reaped.
    /// Reported with a suggested `git worktree remove` command — never removed.
    pub orphaned_worktrees: Vec<String>,
    /// Free-form repo-health flags (branch counts, ahead/behind, uncommitted…).
    pub health_flags: Vec<String>,
    /// Project board to surface the card on. Defaults to Personal if None.
    pub project_id: Option<String>,
}

/// Surface a non-destructive repo-health report as a single board card.
///
/// One card per run — a skimmable markdown summary of the read/report items.
/// Carries structured `metadata_json` (`steward_report: true`) so the board and
/// any later consumer can render it. Read/report only: nothing is proposed or
/// changed. Returns the created card's id.
pub async fn surface_repo_health_report(
    pool: &Pool<Sqlite>,
    report: RepoHealthReport,
) -> Result<String, String> {
    let project_id = report
        .project_id
        .clone()
        .unwrap_or_else(|| crate::projects::PERSONAL_PROJECT_ID.to_string());

    let title = format!("Steward: repo-health report — {}", report.summary);

    let mut description = String::new();
    description.push_str(
        "The Git Steward swept the repository and surfaced these read/report items. \
         This card is informational — nothing here was changed.\n\n",
    );
    description.push_str(&format!("**Repo:** {}\n\n", report.repo_path));
    if !report.stale_merged_branches.is_empty() {
        description.push_str(&format!(
            "**Stale merged branches ({})** — safe to delete; each has its own approval card:\n",
            report.stale_merged_branches.len()
        ));
        for b in &report.stale_merged_branches {
            description.push_str(&format!("- `{}`\n", b));
        }
        description.push('\n');
    }
    if !report.orphaned_worktrees.is_empty() {
        description.push_str(&format!(
            "**Orphaned worktrees ({})** — merged branch, worktree not reaped. \
             Remove with `git worktree remove <path>`:\n",
            report.orphaned_worktrees.len()
        ));
        for w in &report.orphaned_worktrees {
            description.push_str(&format!("- {}\n", w));
        }
        description.push('\n');
    }
    if !report.health_flags.is_empty() {
        description.push_str("**Repo health:**\n");
        for f in &report.health_flags {
            description.push_str(&format!("- {}\n", f));
        }
        description.push('\n');
    }

    let metadata_json = serde_json::json!({
        "steward_report": true,
        "repo_path": report.repo_path,
        "summary": report.summary,
        "stale_merged_branches": report.stale_merged_branches,
        "orphaned_worktrees": report.orphaned_worktrees,
        "health_flags": report.health_flags,
        "needs_human_attention": false,
    });

    let card = cards::create_card(
        pool,
        CreateCard {
            project_id,
            title,
            description: Some(description),
            card_type: Some("standard".to_string()),
            column_id: None,
            created_by: Some("henry".to_string()),
            metadata_json: Some(metadata_json),
        },
    )
    .await?;

    tracing::info!(
        target: "steward",
        card_id = %card.id,
        repo = %report.repo_path,
        "repo-health report surfaced to board"
    );

    Ok(card.id)
}

/// The Git Steward as a scheduled repo-hygiene worker Henry must be able to
/// describe. Surfaced in the `<permagent_self>` brief under "Workers".
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "git_steward",
        display_name: "Git Steward",
        category: crate::agents::self_knowledge::FeatureCategory::Worker,
        what_it_does: "The agent that owns repo hygiene and CI. It runs on its OWN schedule (the \
             `git-steward` recipe — its cron belongs to it, not to you), sweeping repositories \
             to draft commit messages, PR descriptions and changelogs, and to detect stale \
             merged branches and orphaned worktrees (a merged branch whose worktree was \
             never reaped). It surfaces a repo-health card on the board, writes a recallable \
             Brain memory, and BRIEFS YOU directly — both when it proposes destructive \
             cleanup (which becomes an approval card for the user) and when its \
             protected-branch guard refuses an operation outright, which creates no card at \
             all. Destructive work is never run autonomously",
        why_it_matters:
            "It turns invisible repo grind into something you can see, recall, and speak to \
             unprompted. Because it reports to you, you can raise a pending branch deletion \
             or a refused force-push without the user having to go looking at the board. \
             Read/report only — it proposes, the user acts",
        // Worker category ⇒ Queryable by the registry invariant. It carries no
        // bespoke live-state arm in `worker_live_state` (its scheduled run is
        // already reflected in the Scheduler's job count), so it renders
        // editorially with no "now:" claim — honest, not over-claimed.
        state_source: crate::agents::self_knowledge::StateSource::Queryable,
        teaching: &[],
    };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_exact_names() {
        for b in ["main", "master", "develop"] {
            assert!(is_protected_branch(b), "{b} should be protected");
        }
    }

    #[test]
    fn protected_prefixes() {
        for b in [
            "release-1.0",
            "release/2.3",
            "hotfix-urgent",
            "hotfix/login-bug",
        ] {
            assert!(is_protected_branch(b), "{b} should be protected");
        }
    }

    #[test]
    fn protected_with_remote_and_ref_prefixes() {
        assert!(is_protected_branch("origin/main"));
        assert!(is_protected_branch("refs/heads/master"));
        assert!(is_protected_branch("origin/release-1.0"));
    }

    #[test]
    fn non_protected_branches() {
        for b in [
            "feature/foo",
            "fix/bar",
            "research/mesh",
            "my-release-notes", // not a release-* prefix
            "git-steward",
        ] {
            assert!(!is_protected_branch(b), "{b} should NOT be protected");
        }
    }

    #[test]
    fn risk_classification() {
        assert_eq!(classify_risk(GitOpKind::Read), RiskLevel::Low);
        assert_eq!(classify_risk(GitOpKind::BranchCreate), RiskLevel::Low);
        assert_eq!(classify_risk(GitOpKind::LocalCommit), RiskLevel::Medium);
        assert_eq!(classify_risk(GitOpKind::BranchDelete), RiskLevel::High);
        assert_eq!(
            classify_risk(GitOpKind::HistoryRewrite),
            RiskLevel::Critical
        );
        assert_eq!(classify_risk(GitOpKind::ForcePush), RiskLevel::Critical);
    }

    #[test]
    fn routing_autonomous_for_safe_ops() {
        assert_eq!(route(GitOpKind::Read, "anything"), Routing::Autonomous);
        assert_eq!(
            route(GitOpKind::BranchCreate, "feature/x"),
            Routing::Autonomous
        );
        assert_eq!(
            route(GitOpKind::LocalCommit, "feature/x"),
            Routing::Autonomous
        );
    }

    #[test]
    fn routing_requires_approval_for_destructive_on_unprotected() {
        assert_eq!(
            route(GitOpKind::BranchDelete, "feature/stale"),
            Routing::RequiresApproval
        );
        assert_eq!(
            route(GitOpKind::ForcePush, "feature/wip"),
            Routing::RequiresApproval
        );
        assert_eq!(
            route(GitOpKind::HistoryRewrite, "feature/wip"),
            Routing::RequiresApproval
        );
    }

    #[test]
    fn routing_hard_blocks_destructive_on_protected() {
        for branch in ["main", "master", "develop", "release-1.0", "hotfix-x"] {
            assert_eq!(
                route(GitOpKind::BranchDelete, branch),
                Routing::HardBlocked,
                "branch_delete on {branch}"
            );
            assert_eq!(
                route(GitOpKind::ForcePush, branch),
                Routing::HardBlocked,
                "force_push on {branch}"
            );
            assert_eq!(
                route(GitOpKind::HistoryRewrite, branch),
                Routing::HardBlocked,
                "history_rewrite on {branch}"
            );
        }
    }

    #[test]
    fn read_op_on_protected_is_not_blocked() {
        // Reading main is fine; the guard only bites destructive ops.
        assert_eq!(route(GitOpKind::Read, "main"), Routing::Autonomous);
    }

    // ── DB routing path (surface_destructive_proposal) ──

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // Seeds the Personal project and its default Backlog/Doing/Done columns.
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    fn proposal(kind: GitOpKind, branch: &str) -> DestructiveProposal {
        DestructiveProposal {
            kind,
            branch: branch.to_string(),
            command: format!("git branch -D {branch}"),
            reason: "stale 94 days, last commit abc1234, merged to main: yes".to_string(),
            repo_path: "/tmp/repo".to_string(),
            project_id: None,
        }
    }

    async fn card_count(pool: &Pool<Sqlite>) -> usize {
        crate::cards::list_cards(pool, crate::projects::PERSONAL_PROJECT_ID, None, None)
            .await
            .unwrap()
            .len()
    }

    #[tokio::test]
    async fn destructive_on_unprotected_creates_board_card() {
        let pool = test_pool().await;
        let before = card_count(&pool).await;

        let outcome =
            surface_destructive_proposal(&pool, proposal(GitOpKind::BranchDelete, "feature/stale"))
                .await
                .unwrap();

        let card_id = match outcome {
            ProposalOutcome::CardCreated { card_id, risk } => {
                assert_eq!(risk, RiskLevel::High);
                card_id
            }
            other => panic!("expected CardCreated, got {other:?}"),
        };

        assert_eq!(card_count(&pool).await, before + 1, "one card added");

        let card = crate::cards::get_card(&pool, &card_id)
            .await
            .unwrap()
            .expect("card exists");
        assert_eq!(card.created_by, "henry");
        // Metadata must let a human judge from the card alone.
        let meta = &card.metadata_json;
        assert_eq!(meta["steward"], true);
        assert_eq!(meta["op_kind"], "branch_delete");
        assert_eq!(meta["branch"], "feature/stale");
        assert_eq!(meta["risk_tier"], "High");
        assert!(meta["reason"].as_str().unwrap().contains("stale 94 days"));
        assert!(card.description.contains("git branch -D feature/stale"));
    }

    #[tokio::test]
    async fn destructive_on_protected_hard_blocks_with_no_card() {
        let pool = test_pool().await;
        let before = card_count(&pool).await;

        for branch in ["main", "master", "develop", "release-1.0", "hotfix-x"] {
            let outcome =
                surface_destructive_proposal(&pool, proposal(GitOpKind::BranchDelete, branch))
                    .await
                    .unwrap();
            match outcome {
                ProposalOutcome::HardBlocked { reason } => {
                    assert!(reason.contains(branch));
                    assert!(reason.contains("protected"));
                }
                other => panic!("expected HardBlocked for {branch}, got {other:?}"),
            }
        }

        assert_eq!(
            card_count(&pool).await,
            before,
            "protected-branch refusals must create NO cards"
        );
    }

    #[tokio::test]
    async fn force_push_on_protected_hard_blocks() {
        let pool = test_pool().await;
        let outcome = surface_destructive_proposal(&pool, proposal(GitOpKind::ForcePush, "main"))
            .await
            .unwrap();
        assert!(matches!(outcome, ProposalOutcome::HardBlocked { .. }));
        assert_eq!(card_count(&pool).await, 0);
    }

    #[tokio::test]
    async fn non_destructive_makes_no_card() {
        let pool = test_pool().await;
        let outcome = surface_destructive_proposal(&pool, proposal(GitOpKind::Read, "feature/x"))
            .await
            .unwrap();
        assert_eq!(outcome, ProposalOutcome::NotDestructive);
        assert_eq!(card_count(&pool).await, 0);
    }

    // ── Repo-health report (surface_repo_health_report) ──

    #[tokio::test]
    async fn repo_health_report_creates_single_standard_card() {
        let pool = test_pool().await;
        let before = card_count(&pool).await;

        let report = RepoHealthReport {
            repo_path: "/tmp/repo".to_string(),
            summary: "3 stale merged branches, 2 orphaned worktrees".to_string(),
            stale_merged_branches: vec!["feature/old".into(), "fix/done".into(), "chore/x".into()],
            orphaned_worktrees: vec![
                "/tmp/wt/old (feature/old, merged)".into(),
                "/tmp/wt/done (fix/done, merged)".into(),
            ],
            health_flags: vec![
                "897 local branches".into(),
                "HEAD 0 ahead / 0 behind origin/main".into(),
            ],
            project_id: None,
        };

        let card_id = surface_repo_health_report(&pool, report).await.unwrap();

        assert_eq!(
            card_count(&pool).await,
            before + 1,
            "exactly one report card per run"
        );

        let card = crate::cards::get_card(&pool, &card_id)
            .await
            .unwrap()
            .expect("card exists");
        assert_eq!(card.created_by, "henry");
        assert_eq!(card.card_type, "standard");

        let meta = &card.metadata_json;
        assert_eq!(meta["steward_report"], true);
        assert_eq!(meta["needs_human_attention"], false);
        assert_eq!(meta["orphaned_worktrees"].as_array().unwrap().len(), 2);
        assert_eq!(meta["stale_merged_branches"].as_array().unwrap().len(), 3);

        // The body must carry the actionable worktree hint.
        assert!(card.description.contains("Orphaned worktrees"));
        assert!(card.description.contains("git worktree remove"));
        assert!(card.description.contains("feature/old"));
    }
}
