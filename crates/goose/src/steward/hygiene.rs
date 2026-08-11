//! Steward repo-hygiene lane — the ONLY place approved git cleanup executes.
//!
//! Invariant: the Steward detects and proposes; a mutation happens solely in
//! [`apply_repo_hygiene`], which is reachable only from the Decision-Inbox
//! effect arm behind a [`crate::decisions::DecisionProof`] (Tier 2, Jesse-only
//! via the `repo_worktree_reap` / `repo_branch_delete` risk_policy rows).
//!
//! Second-look contract: every safety predicate is RE-VERIFIED at effect time.
//! An approval authorises the *intent*, never the snapshot it was filed from.
//! "The world changed" (target gone, dirty now, unpushed now, unmerged now)
//! resolves `Ok` with a refusal message — the outbox stops retrying because
//! retrying cannot make it safe. "Cannot determine" resolves `Err` — a
//! transient read failure is worth a retry.
//!
//! The worktree primitives ([`ReapOutcome`], [`has_unpushed_work`],
//! [`remove_worktree_dir`]) were lifted verbatim from
//! `agents::platform_extensions::goal_engine` (#504) so the goal reaper and
//! this lane share one implementation; their semantics are unchanged.

use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::decisions::{self, NewDecision, RepoTarget};
use crate::decisions_effects::EffectResult;
use crate::goal_transition::GuardError;
use crate::subprocess::configure_subprocess;
use sqlx::{Pool, Sqlite};

/// risk_policy action class: remove a merged, clean, fully-pushed worktree.
pub const ACTION_REPO_WORKTREE_REAP: &str = "repo_worktree_reap";
/// risk_policy action class: delete a local branch merged into the trunk.
pub const ACTION_REPO_BRANCH_DELETE: &str = "repo_branch_delete";

// ── Lifted #504 primitives (semantics unchanged) ────────────────────────────

/// Outcome of a worktree reap attempt (#504). Returned for logging/observability
/// and asserted in tests; never an error — reaping is best-effort by contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapOutcome {
    /// `git worktree remove` succeeded (git-tracked worktree).
    RemovedTracked,
    /// `git worktree remove` failed; the dir was deleted via `rm -rf` +
    /// `git worktree prune` (an orphaned dir git had lost the ref to).
    RemovedOrphaned,
    /// Nothing on disk to remove.
    Absent,
    /// Kept on purpose: unpushed commits present (or push state unprovable) and
    /// removal was not force-allowed. Protects unreviewed work — see #504.
    SkippedUnpushed,
    /// Removal was attempted but the dir still exists afterwards.
    Failed,
}

/// Run `git <args>` in `dir`; `Some(trimmed stdout)` on a clean exit (possibly
/// empty), `None` if git failed to launch or exited non-zero. This
/// distinguishes "succeeded with empty output" from "failed", which the
/// push-safety guard depends on.
pub(crate) async fn git_checked(dir: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_subprocess(&mut cmd);
    match cmd.output().await {
        Ok(o) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        _ => None,
    }
}

/// Does this worktree hold commits not present on any remote (unpushed work)?
///
/// - `Some(true)`  — HEAD has commits reachable from no remote-tracking ref.
/// - `Some(false)` — HEAD is fully contained on a remote (or has no commits
///   beyond what remotes already have).
/// - `None`        — git state is unreadable (e.g. an orphaned dir whose
///   worktree admin ref is gone). The caller decides: the on-transition reaper
///   protects (can't prove safety); the sweep treats it as safe because an
///   unreadable admin ref means those commits are already unreachable in the
///   repo regardless.
pub(crate) async fn has_unpushed_work(worktree: &Path) -> Option<bool> {
    // Readability probe: if HEAD won't resolve, the worktree's admin ref is gone.
    git_checked(worktree, &["rev-parse", "--verify", "HEAD"]).await?;
    // Commits reachable from HEAD but from no remote-tracking ref.
    let unpushed = git_checked(worktree, &["rev-list", "HEAD", "--not", "--remotes"]).await?;
    Some(!unpushed.is_empty())
}

/// Two-phase removal handling both #504 leak forms. Phase 1 lets git deregister
/// and delete a tracked worktree (`--force` so a dirty working tree of build
/// artifacts doesn't block it). On any failure, phase 2 deletes the dir directly
/// and prunes the stale admin entry — the orphaned-dir case git no longer tracks.
/// A phase-1 failure must never leave the dir behind.
pub(crate) async fn remove_worktree_dir(repo: &Path, dest: &Path) -> ReapOutcome {
    let dest_str = dest.to_string_lossy().to_string();
    if git_checked(repo, &["worktree", "remove", "--force", &dest_str])
        .await
        .is_some()
        && !dest.exists()
    {
        tracing::info!(
            target: "permagentd::brain",
            "reaper: removed tracked worktree {}",
            dest.display()
        );
        return ReapOutcome::RemovedTracked;
    }

    // Orphaned dir (git remove failed / dir survived): rm -rf, then prune the
    // stale `.git/worktrees/<name>` admin entry git may still list.
    let _ = tokio::fs::remove_dir_all(dest).await;
    if dest.exists() {
        tracing::warn!(
            target: "permagentd::brain",
            "reaper: failed to remove worktree dir {}",
            dest.display()
        );
        return ReapOutcome::Failed;
    }
    let _ = git_checked(repo, &["worktree", "prune"]).await;
    tracing::info!(
        target: "permagentd::brain",
        "reaper: removed orphaned worktree dir {} (git ref lost) + pruned",
        dest.display()
    );
    ReapOutcome::RemovedOrphaned
}

// ── Merged-state verdict (honesty precedent: scripts/reap-worktrees.sh) ─────

/// Combine the gh evidence and the ancestry evidence into a merged verdict,
/// RECORDING which path decided (the honesty precedent from
/// `scripts/reap-worktrees.sh`: a squash merge reads UNMERGED to ancestry, so
/// the verdict must say what it actually rests on). `None` = cannot determine.
pub(crate) fn merged_verdict(gh: Option<bool>, ancestor: Option<bool>) -> Option<(bool, String)> {
    match (gh, ancestor) {
        // gh knows about squash merges — a merged PR for this head decides.
        (Some(true), _) => Some((true, "a merged PR for this head (gh, squash-aware)".into())),
        // No merged PR is NOT proof of unmerged (direct pushes exist) — ancestry
        // can still prove merged.
        (_, Some(true)) => Some((true, "merge-base ancestry against the trunk".into())),
        (Some(false), Some(false)) => Some((
            false,
            "no merged PR and not an ancestor of the trunk".into(),
        )),
        (None, Some(false)) => Some((
            false,
            "not an ancestor of the trunk (gh unavailable — a squash merge would read unmerged)"
                .into(),
        )),
        // Ancestry unreadable: gh alone saying "no merged PR" cannot decide.
        (_, None) => None,
    }
}

/// The remote trunk ref to test ancestry against (`origin/<default>`), falling
/// back to `origin/main` when `origin/HEAD` is unset locally.
async fn trunk_ref(repo: &Path) -> String {
    match git_checked(
        repo,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .await
    {
        Some(head) if !head.is_empty() => head,
        _ => "origin/main".to_string(),
    }
}

/// Exit-code-aware ancestry check: `Some(true)`/`Some(false)` are git's answer,
/// `None` means the question itself failed (unknown ref, broken repo).
async fn merge_base_is_ancestor(repo: &Path, branch: &str, trunk: &str) -> Option<bool> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .args(["merge-base", "--is-ancestor", branch, trunk])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_subprocess(&mut cmd);
    match cmd.output().await {
        Ok(o) if o.status.success() => Some(true),
        Ok(o) if o.status.code() == Some(1) => Some(false),
        _ => None,
    }
}

/// Ask gh whether a merged PR exists for this head branch. `None` when gh is
/// unavailable, unauthenticated, or the remote isn't GitHub — never guessed.
async fn gh_merged_pr(repo: &Path, branch: &str) -> Option<bool> {
    let origin = git_checked(repo, &["remote", "get-url", "origin"])
        .await
        .unwrap_or_default();
    if !origin.contains("github.com") {
        return None;
    }
    let mut cmd = Command::new("gh");
    cmd.current_dir(repo)
        .args([
            "pr", "list", "--head", branch, "--state", "merged", "--json", "number",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_subprocess(&mut cmd);
    match cmd.output().await {
        Ok(o) if o.status.success() => {
            let v: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
            Some(v.as_array().is_some_and(|a| !a.is_empty()))
        }
        _ => None,
    }
}

/// Is `branch` merged into the trunk? Tries `gh pr list --state merged` first
/// (squash-merge aware), falls back to `merge-base --is-ancestor`, and records
/// which path decided. `None` = cannot determine right now.
pub async fn branch_is_merged(repo: &Path, branch: &str) -> Option<(bool, String)> {
    let gh = gh_merged_pr(repo, branch).await;
    // Short-circuit: a merged PR decides without touching ancestry.
    if gh == Some(true) {
        return merged_verdict(gh, None).or_else(|| merged_verdict(gh, Some(false)));
    }
    let trunk = trunk_ref(repo).await;
    let ancestor = merge_base_is_ancestor(repo, branch, &trunk).await;
    merged_verdict(gh, ancestor)
}

// ── The effect (second look) ────────────────────────────────────────────────

/// The exact argv for a branch delete. `-d`, NEVER `-D`: git's own merge check
/// is the final guard even after every predicate above passed.
pub(crate) fn branch_delete_args(branch: &str) -> [&'static str; 2] {
    let _ = branch;
    ["branch", "-d"]
}

fn refused(message: impl Into<String>) -> Result<EffectResult, GuardError> {
    // World-changed refusals are Ok: the outbox must stop, not retry —
    // retrying cannot make an unsafe removal safe.
    Ok((Some(message.into()), None))
}

/// Apply an approved repo-hygiene action. Re-verifies EVERY predicate at
/// effect time; see the module docs for the second-look contract.
pub async fn apply_repo_hygiene(
    action_class: &str,
    target: &RepoTarget,
) -> Result<EffectResult, GuardError> {
    match action_class {
        ACTION_REPO_WORKTREE_REAP => apply_worktree_reap(target).await,
        ACTION_REPO_BRANCH_DELETE => apply_branch_delete(target).await,
        other => Err(GuardError::Invalid(format!(
            "unknown repo-hygiene action class '{other}'"
        ))),
    }
}

async fn assert_repo_readable(repo: &Path) -> Result<(), GuardError> {
    if git_checked(repo, &["rev-parse", "--git-dir"])
        .await
        .is_none()
    {
        return Err(GuardError::Invalid(format!(
            "cannot read {} as a git repository right now — nothing removed, will retry",
            repo.display()
        )));
    }
    Ok(())
}

/// Best-effort `git fetch --prune origin` so the merged/pushed checks see fresh
/// refs. Failure is logged and tolerated (the reap-worktrees.sh precedent):
/// stale refs err toward "not merged"/"unpushed", i.e. toward refusing.
async fn fetch_prune(repo: &Path) {
    if git_checked(repo, &["fetch", "--prune", "origin"])
        .await
        .is_none()
    {
        tracing::warn!(
            target: "steward",
            repo = %repo.display(),
            "fetch --prune failed — proceeding on possibly-stale refs (stale errs toward refusal)"
        );
    }
}

async fn apply_worktree_reap(target: &RepoTarget) -> Result<EffectResult, GuardError> {
    // 1. Protected-branch re-assert — independent of any approval surface.
    if let Some(branch) = target.branch.as_deref() {
        if super::is_protected_branch(branch) {
            return Err(GuardError::Invalid(format!(
                "REFUSED: '{branch}' is a protected branch — the Steward never removes its \
                 worktree, an approval notwithstanding"
            )));
        }
    }
    let Some(wt) = target.worktree_path.as_deref() else {
        return Err(GuardError::Invalid(
            "repo_worktree_reap target has no worktree_path — nothing safe to apply".to_string(),
        ));
    };
    let repo = Path::new(&target.repo_path);
    let wt_path = Path::new(wt);

    // 2. The repo must still be a repo.
    assert_repo_readable(repo).await?;
    // 3. Fresh refs for the pushed/merged checks (best-effort).
    fetch_prune(repo).await;
    // 4. Already gone → the world already looks like the approved outcome.
    if !wt_path.exists() {
        return refused(format!("worktree {wt} is already gone — nothing to remove"));
    }
    // 5. Must still be a REGISTERED worktree of this repo — an unregistered
    //    directory at that path could be anything; not touched.
    let listing = git_checked(repo, &["worktree", "list", "--porcelain"])
        .await
        .ok_or_else(|| {
            GuardError::Invalid(
                "cannot read `git worktree list` right now — nothing removed, will retry"
                    .to_string(),
            )
        })?;
    let entries = super::git_health::parse_worktree_list_porcelain(&listing);
    // Compare canonicalized paths: `git worktree list` prints fully-resolved
    // paths (/private/var/… on macOS) while the approval may carry the symlink
    // form (/var/…) — a literal compare would mis-read a registered worktree
    // as unregistered and refuse a legitimate reap.
    let wt_canon = std::fs::canonicalize(wt_path).unwrap_or_else(|_| wt_path.to_path_buf());
    let Some(entry) = entries.iter().find(|e| {
        let p = Path::new(&e.path);
        p == wt_path
            || std::fs::canonicalize(p)
                .map(|c| c == wt_canon)
                .unwrap_or(false)
    }) else {
        return refused(format!(
            "{wt} is no longer a registered worktree of this repo — refusing to touch the \
             directory"
        ));
    };
    // Branch drift: approved-for one branch, now on another = world changed.
    if let (Some(approved), Some(now)) = (target.branch.as_deref(), entry.branch.as_deref()) {
        if approved != now {
            return refused(format!(
                "worktree {wt} switched from '{approved}' to '{now}' since approval — refused, \
                 nothing removed"
            ));
        }
    }
    // 6. Clean NOW.
    let status = git_checked(wt_path, &["status", "--porcelain"])
        .await
        .ok_or_else(|| {
            GuardError::Invalid(format!(
                "cannot read the working-tree status of {wt} — nothing removed, will retry"
            ))
        })?;
    if !status.is_empty() {
        return refused(format!(
            "worktree {wt} has uncommitted changes now — refused, nothing removed"
        ));
    }
    // 7. Fully pushed NOW. Some(true)=refuse, None=Err (cannot prove safety).
    match has_unpushed_work(wt_path).await {
        Some(true) => {
            return refused(format!(
                "worktree {wt} has unpushed commits now — refused, nothing removed"
            ))
        }
        None => {
            return Err(GuardError::Invalid(format!(
                "cannot determine the push state of {wt} — nothing removed, will retry"
            )))
        }
        Some(false) => {}
    }
    // 8. Still merged NOW (when a branch was named; detached goal worktrees
    //    carry none and are guarded by the pushed check above).
    let mut merged_note = String::new();
    if let Some(branch) = target.branch.as_deref() {
        match branch_is_merged(repo, branch).await {
            Some((true, via)) => merged_note = format!("; merged per {via}"),
            Some((false, via)) => {
                return refused(format!(
                    "'{branch}' no longer reads as merged ({via}) — refused, nothing removed"
                ))
            }
            None => {
                return Err(GuardError::Invalid(format!(
                    "cannot determine the merge state of '{branch}' — nothing removed, will retry"
                )))
            }
        }
    }
    // 9. Two-phase removal.
    match remove_worktree_dir(repo, wt_path).await {
        ReapOutcome::RemovedTracked => Ok((
            Some(format!("worktree {wt} removed (git-tracked{merged_note})")),
            None,
        )),
        ReapOutcome::RemovedOrphaned => Ok((
            Some(format!(
                "worktree {wt} removed (orphaned dir, admin entry pruned{merged_note})"
            )),
            None,
        )),
        ReapOutcome::Absent => refused(format!("worktree {wt} was already gone")),
        ReapOutcome::SkippedUnpushed => refused(format!(
            "worktree {wt} kept — unpushed work surfaced during removal"
        )),
        ReapOutcome::Failed => Err(GuardError::Invalid(format!(
            "removal attempted but {wt} still exists — will retry"
        ))),
    }
}

async fn apply_branch_delete(target: &RepoTarget) -> Result<EffectResult, GuardError> {
    let Some(branch) = target.branch.as_deref() else {
        return Err(GuardError::Invalid(
            "repo_branch_delete target has no branch — nothing safe to apply".to_string(),
        ));
    };
    // 1. Protected-branch re-assert — independent of any approval surface.
    if super::is_protected_branch(branch) {
        return Err(GuardError::Invalid(format!(
            "REFUSED: '{branch}' is a protected branch — the Steward never deletes it, an \
             approval notwithstanding"
        )));
    }
    let repo = Path::new(&target.repo_path);
    assert_repo_readable(repo).await?;
    fetch_prune(repo).await;

    // 2. Ref still exists → absent means the approved outcome already holds.
    let head_ref = format!("refs/heads/{branch}");
    if git_checked(repo, &["rev-parse", "--verify", "--quiet", &head_ref])
        .await
        .is_none()
    {
        return refused(format!(
            "branch '{branch}' is already gone — nothing to delete"
        ));
    }
    // 3. Not checked out in any worktree NOW.
    let listing = git_checked(repo, &["worktree", "list", "--porcelain"])
        .await
        .ok_or_else(|| {
            GuardError::Invalid(
                "cannot read `git worktree list` right now — nothing deleted, will retry"
                    .to_string(),
            )
        })?;
    if super::git_health::parse_worktree_list_porcelain(&listing)
        .iter()
        .any(|e| e.branch.as_deref() == Some(branch))
    {
        return refused(format!(
            "branch '{branch}' is checked out in a worktree now — refused, nothing deleted"
        ));
    }
    // 4. Still merged NOW.
    let via = match branch_is_merged(repo, branch).await {
        Some((true, via)) => via,
        Some((false, via)) => {
            return refused(format!(
                "'{branch}' no longer reads as merged ({via}) — refused, nothing deleted"
            ))
        }
        None => {
            return Err(GuardError::Invalid(format!(
                "cannot determine the merge state of '{branch}' — nothing deleted, will retry"
            )))
        }
    };
    // 5. Delete with -d, NEVER -D: git's own merge check is the last guard.
    let args = branch_delete_args(branch);
    if git_checked(repo, &[args[0], args[1], branch])
        .await
        .is_none()
    {
        return Err(GuardError::Invalid(format!(
            "`git branch -d {branch}` refused (git's own merge check is the final guard) — \
             nothing deleted, will retry"
        )));
    }
    Ok((
        Some(format!(
            "branch '{branch}' deleted (merged per {via}; git -d merge check passed)"
        )),
        None,
    ))
}

// ── The proposer (detect → Decision Inbox, never acts) ──────────────────────

/// A repo-hygiene cleanup the sweep wants to pitch as a Tier-2 decision.
#[derive(Debug, Clone)]
pub struct RepoHygieneProposal {
    /// [`ACTION_REPO_WORKTREE_REAP`] or [`ACTION_REPO_BRANCH_DELETE`].
    pub action_class: String,
    pub repo_path: String,
    pub worktree_path: Option<String>,
    pub branch: Option<String>,
    /// Evidence lines shown to the approver (branch, sha, merged-via, age).
    pub evidence: Vec<String>,
    /// Plain-language headline, ≤80 chars (truncated if longer).
    pub headline: String,
    pub project_id: Option<String>,
}

/// File a `risk_gate` decision for a repo-hygiene cleanup. Returns the decision
/// id, or `None` when the anti-nag guard skipped it: the same target already
/// has an OPEN decision, or was previously REJECTED (a "no" is remembered —
/// never re-pitched).
pub async fn propose_repo_hygiene(
    pool: &Pool<Sqlite>,
    proposal: RepoHygieneProposal,
) -> Result<Option<String>, String> {
    let dup: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM decisions
         WHERE kind = 'risk_gate'
           AND json_extract(payload_json, '$.action_class') = ?
           AND (status = 'open' OR answer = 'reject')
           AND COALESCE(json_extract(payload_json, '$.repo_target.worktree_path'), '') = ?
           AND COALESCE(json_extract(payload_json, '$.repo_target.branch'), '') = ?",
    )
    .bind(&proposal.action_class)
    .bind(proposal.worktree_path.as_deref().unwrap_or(""))
    .bind(proposal.branch.as_deref().unwrap_or(""))
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    if dup > 0 {
        tracing::debug!(
            target: "steward",
            action = %proposal.action_class,
            worktree = %proposal.worktree_path.as_deref().unwrap_or("-"),
            branch = %proposal.branch.as_deref().unwrap_or("-"),
            "anti-nag: open or previously-rejected decision for this target — not re-pitched"
        );
        return Ok(None);
    }

    let headline: String = if proposal.headline.chars().count() > decisions::MAX_HEADLINE_CHARS {
        let mut h: String = proposal
            .headline
            .chars()
            .take(decisions::MAX_HEADLINE_CHARS - 1)
            .collect();
        h.push('…');
        h
    } else {
        proposal.headline.clone()
    };
    let detail = if proposal.evidence.is_empty() {
        format!("Repo: {}", proposal.repo_path)
    } else {
        proposal.evidence.join("\n")
    };
    let description = match proposal.action_class.as_str() {
        ACTION_REPO_WORKTREE_REAP => "Remove a merged, clean, fully-pushed worktree directory",
        ACTION_REPO_BRANCH_DELETE => "Delete a local branch merged into the trunk",
        other => other,
    };
    let payload = serde_json::json!({
        "action_class": proposal.action_class,
        "description": description,
        "requested_by": "steward",
        "repo_target": {
            "repo_path": proposal.repo_path,
            "worktree_path": proposal.worktree_path,
            "branch": proposal.branch,
            "evidence": proposal.evidence,
        },
    });
    let d = decisions::create_decision(
        pool,
        NewDecision {
            kind: "risk_gate".to_string(),
            project_id: proposal.project_id,
            headline: Some(headline),
            detail: Some(detail),
            payload,
            ..Default::default()
        },
    )
    .await?;
    if d.kind == "malformed" {
        tracing::warn!(
            target: "steward",
            decision = %d.id,
            "repo-hygiene proposal was stored as malformed — check the payload shape"
        );
    }
    Ok(Some(d.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn git(dir: &Path, args: &[&str]) -> std::process::Output {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
    }

    /// Repo at `<tmp>/proj` with a file:// `origin` and one baseline commit
    /// pushed to `origin/main` (same fixture shape as goal_engine's #504 tests).
    fn init_repo_with_remote(tmp: &Path) -> (PathBuf, String) {
        let origin = tmp.join("origin.git");
        std::process::Command::new("git")
            .args(["init", "-q", "--bare"])
            .arg(&origin)
            .output()
            .unwrap();
        let repo = tmp.join("proj");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@t.t"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("README.md"), "hi").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        git(&repo, &["branch", "-M", "main"]);
        git(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git(&repo, &["push", "-q", "-u", "origin", "main"]);
        git(&repo, &["fetch", "-q", "origin"]);
        let head = String::from_utf8(git(&repo, &["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        (repo, head)
    }

    fn add_worktree(repo: &Path, tmp: &Path, name: &str, branch: Option<&str>) -> PathBuf {
        let wt = tmp.join(name);
        match branch {
            Some(b) => git(
                repo,
                &["worktree", "add", "-q", "-b", b, wt.to_str().unwrap()],
            ),
            None => git(
                repo,
                &["worktree", "add", "-q", "--detach", wt.to_str().unwrap()],
            ),
        };
        assert!(wt.is_dir(), "worktree fixture must exist");
        wt
    }

    fn target(repo: &Path, wt: Option<&Path>, branch: Option<&str>) -> RepoTarget {
        RepoTarget {
            repo_path: repo.to_string_lossy().to_string(),
            worktree_path: wt.map(|p| p.to_string_lossy().to_string()),
            branch: branch.map(str::to_string),
            evidence: vec![],
        }
    }

    // ── Refusal table ──

    #[tokio::test]
    async fn protected_branch_approved_anyway_is_err_refused() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, _) = init_repo_with_remote(tmp.path());
        for class in [ACTION_REPO_BRANCH_DELETE, ACTION_REPO_WORKTREE_REAP] {
            let t = target(&repo, Some(&repo.join("x")), Some("main"));
            let err = apply_repo_hygiene(class, &t).await.unwrap_err();
            assert!(
                format!("{err:?}").contains("REFUSED"),
                "{class}: protected branch must be a hard Err refusal, got {err:?}"
            );
        }
        // And main survives.
        assert!(git(&repo, &["rev-parse", "--verify", "refs/heads/main"])
            .status
            .success());
    }

    #[tokio::test]
    async fn vanished_worktree_reads_already_applied() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, _) = init_repo_with_remote(tmp.path());
        let t = target(&repo, Some(&tmp.path().join("never-existed")), None);
        let (msg, _) = apply_repo_hygiene(ACTION_REPO_WORKTREE_REAP, &t)
            .await
            .unwrap();
        assert!(msg.unwrap().contains("already gone"));
    }

    #[tokio::test]
    async fn dirty_now_refuses_ok_and_keeps_the_worktree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, _) = init_repo_with_remote(tmp.path());
        let wt = add_worktree(&repo, tmp.path(), "wt-dirty", None);
        std::fs::write(wt.join("scratch.txt"), "uncommitted").unwrap();
        let t = target(&repo, Some(&wt), None);
        let (msg, _) = apply_repo_hygiene(ACTION_REPO_WORKTREE_REAP, &t)
            .await
            .unwrap();
        assert!(msg.unwrap().contains("uncommitted changes now"));
        assert!(wt.exists(), "a dirty worktree must never be removed");
    }

    #[tokio::test]
    async fn unpushed_now_refuses_ok_and_keeps_the_worktree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, _) = init_repo_with_remote(tmp.path());
        let wt = add_worktree(&repo, tmp.path(), "wt-unpushed", None);
        std::fs::write(wt.join("work.txt"), "work").unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-q", "-m", "unpushed work"]);
        let t = target(&repo, Some(&wt), None);
        let (msg, _) = apply_repo_hygiene(ACTION_REPO_WORKTREE_REAP, &t)
            .await
            .unwrap();
        assert!(msg.unwrap().contains("unpushed commits now"));
        assert!(wt.exists(), "unpushed work must never be destroyed");
    }

    #[tokio::test]
    async fn unreadable_repo_is_err_for_retry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let not_a_repo = tmp.path().join("plain-dir");
        std::fs::create_dir(&not_a_repo).unwrap();
        let t = target(&not_a_repo, Some(&not_a_repo.join("wt")), None);
        assert!(apply_repo_hygiene(ACTION_REPO_WORKTREE_REAP, &t)
            .await
            .is_err());
        let t2 = target(&not_a_repo, None, Some("feature/x"));
        assert!(apply_repo_hygiene(ACTION_REPO_BRANCH_DELETE, &t2)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn unregistered_directory_is_not_touched() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, _) = init_repo_with_remote(tmp.path());
        let dir = tmp.path().join("random-dir");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("data.txt"), "not a worktree").unwrap();
        let t = target(&repo, Some(&dir), None);
        let (msg, _) = apply_repo_hygiene(ACTION_REPO_WORKTREE_REAP, &t)
            .await
            .unwrap();
        assert!(msg.unwrap().contains("no longer a registered worktree"));
        assert!(
            dir.exists(),
            "an unregistered directory must never be touched"
        );
    }

    // ── Happy paths ──

    #[tokio::test]
    async fn clean_pushed_worktree_is_removed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, _) = init_repo_with_remote(tmp.path());
        let wt = add_worktree(&repo, tmp.path(), "wt-clean", None);
        let t = target(&repo, Some(&wt), None);
        let (msg, _) = apply_repo_hygiene(ACTION_REPO_WORKTREE_REAP, &t)
            .await
            .unwrap();
        assert!(msg.unwrap().contains("removed"));
        assert!(!wt.exists());
    }

    #[tokio::test]
    async fn merged_branch_is_deleted_with_dash_d_and_honesty_string() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, _) = init_repo_with_remote(tmp.path());
        // A branch at the baseline commit is trivially an ancestor of
        // origin/main — merged by ancestry (file:// origin, so gh never runs).
        git(&repo, &["branch", "feature/done"]);
        let t = target(&repo, None, Some("feature/done"));
        let (msg, _) = apply_repo_hygiene(ACTION_REPO_BRANCH_DELETE, &t)
            .await
            .unwrap();
        let msg = msg.unwrap();
        assert!(msg.contains("deleted"));
        assert!(
            msg.contains("merge-base ancestry"),
            "must record WHICH path decided merged-ness: {msg}"
        );
        assert!(
            !git(&repo, &["rev-parse", "--verify", "refs/heads/feature/done"])
                .status
                .success()
        );
    }

    #[tokio::test]
    async fn unmerged_branch_refuses_ok_and_survives() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, _) = init_repo_with_remote(tmp.path());
        git(&repo, &["checkout", "-q", "-b", "feature/wip"]);
        std::fs::write(repo.join("wip.txt"), "wip").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "unmerged work"]);
        git(&repo, &["checkout", "-q", "main"]);
        let t = target(&repo, None, Some("feature/wip"));
        let (msg, _) = apply_repo_hygiene(ACTION_REPO_BRANCH_DELETE, &t)
            .await
            .unwrap();
        assert!(msg.unwrap().contains("no longer reads as merged"));
        assert!(
            git(&repo, &["rev-parse", "--verify", "refs/heads/feature/wip"])
                .status
                .success()
        );
    }

    #[tokio::test]
    async fn checked_out_branch_refuses_ok() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, _) = init_repo_with_remote(tmp.path());
        let _wt = add_worktree(&repo, tmp.path(), "wt-co", Some("feature/live"));
        let t = target(&repo, None, Some("feature/live"));
        let (msg, _) = apply_repo_hygiene(ACTION_REPO_BRANCH_DELETE, &t)
            .await
            .unwrap();
        assert!(msg.unwrap().contains("checked out in a worktree"));
        assert!(
            git(&repo, &["rev-parse", "--verify", "refs/heads/feature/live"])
                .status
                .success()
        );
    }

    #[tokio::test]
    async fn already_deleted_branch_reads_already_applied() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, _) = init_repo_with_remote(tmp.path());
        let t = target(&repo, None, Some("feature/gone"));
        let (msg, _) = apply_repo_hygiene(ACTION_REPO_BRANCH_DELETE, &t)
            .await
            .unwrap();
        assert!(msg.unwrap().contains("already gone"));
    }

    // ── -d, never -D ──

    #[test]
    fn branch_delete_argv_is_lowercase_d_never_capital() {
        let args = branch_delete_args("feature/anything");
        assert_eq!(args, ["branch", "-d"]);
        assert!(
            !args.contains(&"-D"),
            "the Steward must NEVER force-delete a branch"
        );
    }

    // ── Merged-verdict honesty ──

    #[test]
    fn merged_verdict_records_which_path_decided() {
        // gh decides (squash-aware) even when ancestry disagrees.
        let (m, via) = merged_verdict(Some(true), Some(false)).unwrap();
        assert!(m);
        assert!(via.contains("gh"), "gh path must be named: {via}");
        // Ancestry fallback names itself.
        let (m, via) = merged_verdict(None, Some(true)).unwrap();
        assert!(m);
        assert!(via.contains("merge-base ancestry"), "{via}");
        // Unmerged with gh silent states the squash-merge caveat honestly.
        let (m, via) = merged_verdict(None, Some(false)).unwrap();
        assert!(!m);
        assert!(via.contains("gh unavailable"), "{via}");
        // Both readable and both no → definite unmerged.
        let (m, via) = merged_verdict(Some(false), Some(false)).unwrap();
        assert!(!m);
        assert!(via.contains("no merged PR"), "{via}");
        // Nothing readable → cannot determine.
        assert!(merged_verdict(None, None).is_none());
        assert!(merged_verdict(Some(false), None).is_none());
    }

    // ── Anti-nag proposer ──

    async fn test_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        pool
    }

    fn proposal(branch: &str) -> RepoHygieneProposal {
        RepoHygieneProposal {
            action_class: ACTION_REPO_BRANCH_DELETE.to_string(),
            repo_path: "/tmp/proj".to_string(),
            worktree_path: None,
            branch: Some(branch.to_string()),
            evidence: vec![
                format!("branch: {branch}"),
                "sha: abc1234".to_string(),
                "merged via: merge-base ancestry".to_string(),
                "age: 41 days".to_string(),
            ],
            headline: format!("Tidy up: delete the merged branch {branch}?"),
            project_id: None,
        }
    }

    #[tokio::test]
    async fn proposer_files_a_tier2_risk_gate_with_repo_target() {
        let pool = test_pool().await;
        let id = propose_repo_hygiene(&pool, proposal("feature/old"))
            .await
            .unwrap()
            .expect("first pitch files");
        let d = decisions::get_decision(&pool, &id).await.unwrap().unwrap();
        assert_eq!(d.kind, "risk_gate");
        assert_eq!(d.tier, 2, "repo hygiene must be Jesse-only");
        let payload: decisions::RiskGatePayload =
            serde_json::from_value(d.payload.clone()).unwrap();
        assert_eq!(payload.action_class, ACTION_REPO_BRANCH_DELETE);
        assert_eq!(payload.requested_by, "steward");
        let t = payload.repo_target.expect("repo_target present");
        assert_eq!(t.branch.as_deref(), Some("feature/old"));
        assert_eq!(t.evidence.len(), 4);
        assert!(d.detail.contains("merged via"));
    }

    #[tokio::test]
    async fn open_decision_for_same_target_is_not_repitched() {
        let pool = test_pool().await;
        assert!(propose_repo_hygiene(&pool, proposal("feature/dup"))
            .await
            .unwrap()
            .is_some());
        assert!(
            propose_repo_hygiene(&pool, proposal("feature/dup"))
                .await
                .unwrap()
                .is_none(),
            "anti-nag: an open decision for the target blocks a re-pitch"
        );
        // A different target still files.
        assert!(propose_repo_hygiene(&pool, proposal("feature/other"))
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn rejected_target_is_never_repitched() {
        let pool = test_pool().await;
        let id = propose_repo_hygiene(&pool, proposal("feature/no"))
            .await
            .unwrap()
            .unwrap();
        decisions::answer_decision(
            &pool,
            &id,
            &decisions::DecisionAnswer {
                answer: "reject".to_string(),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        assert!(
            propose_repo_hygiene(&pool, proposal("feature/no"))
                .await
                .unwrap()
                .is_none(),
            "anti-nag: a rejected target is remembered, never re-pitched"
        );
    }
}
