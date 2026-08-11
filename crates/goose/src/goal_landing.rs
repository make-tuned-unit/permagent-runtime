//! Landing approved goal work back onto the project's trunk.
//!
//! Workers commit inside an isolated worktree on a `goal/<run_id>` branch and
//! are told never to push. Until now approval stopped there: the card moved
//! Review → Complete and the commits stayed on the branch. Measured end to end
//! on 2026-08-09 — approve returned `"goal approved: Review → Complete"`, and
//! `main` was byte-identical with the file absent. Every downstream goal in a
//! roadmap therefore starts from a trunk that never received its dependency's
//! output, which is the concrete reason a DAG of goals cannot flow.
//!
//! ## Fast-forward only, on purpose
//!
//! Landing runs inside the decision-answer request. A real merge can conflict,
//! and resolving a conflict is neither a thing to attempt unattended nor a
//! thing to do while a user waits on an HTTP response. So this fast-forwards
//! when the trunk has not moved since the worker branched, and otherwise
//! reports precisely that, leaving the branch intact for a human or a rebase.
//! A refusal is never a silent no-op: the outcome is surfaced on the approval
//! response, because "approved" that did nothing is the bug this replaces.
//!
//! Local only — nothing here pushes. The remote is a separate decision.

use std::path::Path;
use std::process::Stdio;

/// What landing did, or honestly why it did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandOutcome {
    /// Trunk fast-forwarded onto the worker's head.
    FastForwarded {
        branch: String,
        trunk: String,
        head: String,
    },
    /// The trunk moved since the worker branched; landing needs a merge or
    /// rebase a human should see.
    NeedsMerge { branch: String, trunk: String },
    /// Trunk already contains the work (a re-approval, or someone merged it).
    AlreadyLanded { trunk: String },
    /// Nothing to land — the worker committed nothing.
    NoCommits,
    /// Landing was not attempted; the string says why.
    Skipped(String),
}

impl LandOutcome {
    /// One line for the approval response. Every variant reports something:
    /// silence is what made the old behaviour indistinguishable from success.
    pub fn describe(&self) -> String {
        match self {
            Self::FastForwarded {
                branch,
                trunk,
                head,
            } => {
                // chars().take, not a byte slice: commit ids are ASCII today,
                // but the workspace bans string indexing outright
                // (clippy::string_slice) — see df5d0d567 for the same fix.
                let short: String = head.chars().take(9).collect();
                format!("landed: {} fast-forwarded to {} ({})", trunk, short, branch)
            }
            Self::NeedsMerge { branch, trunk } => format!(
                "NOT landed: {} has moved since the worker branched, so {} cannot fast-forward. \
                 The work is intact on that branch — merge or rebase it.",
                trunk, branch
            ),
            Self::AlreadyLanded { trunk } => {
                format!("already landed: {} already contains this work", trunk)
            }
            Self::NoCommits => "nothing to land: the worker produced no commits".to_string(),
            Self::Skipped(why) => format!("NOT landed: {}", why),
        }
    }

    /// True when the trunk now contains the work.
    pub fn is_landed(&self) -> bool {
        matches!(
            self,
            Self::FastForwarded { .. } | Self::AlreadyLanded { .. }
        )
    }
}

/// The inputs landing needs, read off the goal card's `dispatch_evidence`.
#[derive(Debug, Clone)]
pub struct LandRequest {
    /// The project's repo root — where the trunk is checked out.
    pub repo_root: String,
    /// `goal/<run_id>` — the branch the worker committed on.
    pub branch: String,
    /// The worker's true head commit.
    pub head_commit: String,
}

/// Pull the landing inputs out of a goal card's metadata.
///
/// The branch is derived from the worktree directory name rather than stored:
/// `ExternalCliEngine` names both from the same run id, and a value re-derived
/// from the path cannot drift from a separately-persisted copy.
pub fn land_request_from_metadata(
    metadata: &serde_json::Value,
    repo_root: Option<&str>,
) -> Option<LandRequest> {
    let evidence = metadata.get("dispatch_evidence")?;
    let head_commit = evidence.get("head_commit")?.as_str()?.to_string();
    if head_commit.is_empty() {
        return None;
    }
    if evidence
        .get("commits")
        .and_then(|c| c.as_array())
        .is_some_and(|c| c.is_empty())
    {
        return None;
    }
    let worktree = evidence.get("worktree_path")?.as_str()?;
    let run_id = Path::new(worktree).file_name()?.to_str()?;
    Some(LandRequest {
        repo_root: repo_root?.to_string(),
        branch: format!("goal/{}", run_id),
        head_commit,
    })
}

async fn git(root: &str, args: &[&str]) -> Option<(bool, String)> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some((out.status.success(), text))
}

/// Fast-forward the project's trunk onto an approved goal's work.
pub async fn land(req: &LandRequest) -> LandOutcome {
    let root = req.repo_root.as_str();

    let Some((ok, trunk)) = git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).await else {
        return LandOutcome::Skipped("could not run git in the project root".to_string());
    };
    if !ok || trunk.is_empty() || trunk == "HEAD" {
        return LandOutcome::Skipped(
            "the project root is not on a branch (detached HEAD) — landing needs a checked-out trunk"
                .to_string(),
        );
    }

    // Never fast-forward over uncommitted work: `merge --ff-only` can overwrite
    // a dirty tree's files, and the user's in-progress edits are not ours to
    // move.
    match git(root, &["status", "--porcelain"]).await {
        Some((_, dirty)) if !dirty.is_empty() => {
            return LandOutcome::Skipped(format!(
                "the project root has uncommitted changes, so {} was left untouched. \
                 Commit or stash them, then merge {}.",
                trunk, req.branch
            ));
        }
        None => return LandOutcome::Skipped("could not read git status".to_string()),
        _ => {}
    }

    // Already contains it? Then approval is idempotent rather than an error.
    if let Some((true, _)) = git(
        root,
        &["merge-base", "--is-ancestor", &req.head_commit, "HEAD"],
    )
    .await
    {
        return LandOutcome::AlreadyLanded { trunk };
    }

    // Fast-forwardable exactly when the trunk is an ancestor of the work.
    let ff = git(
        root,
        &["merge-base", "--is-ancestor", "HEAD", &req.head_commit],
    )
    .await;
    if !matches!(ff, Some((true, _))) {
        return LandOutcome::NeedsMerge {
            branch: req.branch.clone(),
            trunk,
        };
    }

    match git(root, &["merge", "--ff-only", &req.head_commit]).await {
        Some((true, _)) => LandOutcome::FastForwarded {
            branch: req.branch.clone(),
            trunk,
            head: req.head_commit.clone(),
        },
        Some((false, _)) | None => LandOutcome::NeedsMerge {
            branch: req.branch.clone(),
            trunk,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(worktree: &str, head: &str, commits: usize) -> serde_json::Value {
        serde_json::json!({
            "dispatch_evidence": {
                "worktree_path": worktree,
                "head_commit": head,
                "commits": vec!["abc123 did a thing"; commits],
            }
        })
    }

    #[test]
    fn branch_is_derived_from_the_worktree_directory_name() {
        let meta = evidence("/dev/.permagent-goal-worktrees/cli-abc-123", "deadbeef", 1);
        let req = land_request_from_metadata(&meta, Some("/dev/proj")).unwrap();
        assert_eq!(req.branch, "goal/cli-abc-123");
        assert_eq!(req.head_commit, "deadbeef");
        assert_eq!(req.repo_root, "/dev/proj");
    }

    #[test]
    fn a_worker_that_committed_nothing_has_nothing_to_land() {
        let meta = evidence("/wt/cli-1", "deadbeef", 0);
        assert!(land_request_from_metadata(&meta, Some("/dev/proj")).is_none());
    }

    /// A project with no root_path has nowhere to land — the old code would
    /// have fallen back to the daemon's cwd, which is not the user's repo.
    #[test]
    fn no_repo_root_means_no_landing() {
        let meta = evidence("/wt/cli-1", "deadbeef", 1);
        assert!(land_request_from_metadata(&meta, None).is_none());
    }

    #[test]
    fn every_outcome_says_something_specific() {
        let ff = LandOutcome::FastForwarded {
            branch: "goal/cli-1".into(),
            trunk: "main".into(),
            head: "deadbeefcafe".into(),
        };
        assert!(ff.describe().contains("main") && ff.describe().contains("deadbeefc"));
        assert!(ff.is_landed());

        let needs = LandOutcome::NeedsMerge {
            branch: "goal/cli-1".into(),
            trunk: "main".into(),
        };
        assert!(needs.describe().contains("intact"), "{}", needs.describe());
        assert!(!needs.is_landed(), "a refusal must not read as landed");

        assert!(LandOutcome::NoCommits.describe().contains("no commits"));
        assert!(LandOutcome::AlreadyLanded {
            trunk: "main".into()
        }
        .is_landed());
    }
}
