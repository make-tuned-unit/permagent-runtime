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

    /// A short, stable key for persisting this outcome on a goal card (YF2,
    /// G6 review, Lane Y-fix 2026-09-09) — `dispatch_evidence.landing.outcome`
    /// — so a later reader (`goal_landed_head_commit`, the CLI drain
    /// summary) can tell "landing was attempted and actually succeeded"
    /// apart from "attempted but did nothing" without re-deriving it from
    /// the variant's prose.
    pub fn outcome_key(&self) -> &'static str {
        if self.is_landed() {
            "landed"
        } else if matches!(self, Self::NeedsMerge { .. }) {
            "needs_merge"
        } else {
            "skipped"
        }
    }

    /// The trunk sha the work actually landed at, when it did.
    pub fn landed_head(&self) -> Option<&str> {
        match self {
            Self::FastForwarded { head, .. } => Some(head.as_str()),
            Self::AlreadyLanded { trunk } => Some(trunk.as_str()),
            _ => None,
        }
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

/// The two dispatch-evidence fields landing needs that do NOT depend on
/// where the project's trunk lives: the worktree the worker committed in,
/// and its true head commit. Split out of [`land_request_from_metadata`]
/// (R1, harness loop Lane D1) so a caller can tell "the worker committed
/// nothing landable" apart from "there is real work but no repo root was
/// resolved" without duplicating these emptiness checks — `land_approved_goal`
/// needs exactly that distinction to know whether a missing `root_path`
/// deserves an honest failure outcome or is simply nothing to report.
pub(crate) fn evidence_head_and_worktree(metadata: &serde_json::Value) -> Option<(String, String)> {
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
    let worktree = evidence.get("worktree_path")?.as_str()?.to_string();
    Some((worktree, head_commit))
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
    let (worktree, head_commit) = evidence_head_and_worktree(metadata)?;
    let run_id = Path::new(&worktree).file_name()?.to_str()?.to_string();
    Some(LandRequest {
        repo_root: repo_root?.to_string(),
        branch: format!("goal/{}", run_id),
        head_commit,
    })
}

/// R1 (harness loop, Lane D1, from Lane F8 forensics): recover the primary
/// checkout a goal's worktree branched off, for when the project's own
/// `root_path` was never set (the eval harness's seeded `personal` project,
/// `session/spectral_schema.rs:610`, and any other project created with no
/// local folder). `git worktree add` (`goal_engine::create_goal_worktree`)
/// always leaves the linked worktree's `.git` FILE naming the shared git dir
/// that lives inside the checkout that created it — so asking git, from
/// inside the worktree, for that common dir and taking its parent recovers
/// the exact directory `resolve_internal_isolation`/`create_goal_worktree`
/// branched from, without needing it stored separately on the card or
/// guessed from the worktree path's own shape (which cannot work: the
/// worktree lives beside the repo, `<repo>/../.permagent-goal-worktrees/<run_id>`,
/// not under it — the repo's own directory name is not recoverable from the
/// worktree path alone).
///
/// `None` when the worktree no longer exists, is not a git worktree at all,
/// or its common dir has no parent (a git dir at `/`, never a real repo).
pub(crate) async fn resolve_primary_checkout_from_worktree(worktree_path: &str) -> Option<String> {
    if !Path::new(worktree_path).is_dir() {
        return None;
    }
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(worktree_path)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .stdin(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let common_git_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if common_git_dir.is_empty() {
        return None;
    }
    let checkout = Path::new(&common_git_dir).parent()?;
    if !checkout.is_dir() {
        return None;
    }
    Some(checkout.to_string_lossy().into_owned())
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

    fn git_in(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// R1 (harness loop, Lane D1): a linked worktree's `.git` FILE always
    /// names the shared git dir inside the checkout that created it — the
    /// same mechanism `land_approved_goal` leans on when a project's
    /// `root_path` is NULL (Lane F8 forensics: the eval harness's default
    /// `personal` project is seeded that way).
    #[tokio::test]
    async fn resolve_primary_checkout_from_worktree_finds_the_repo_that_created_it() {
        let repo = tempfile::tempdir().unwrap();
        git_in(repo.path(), &["init", "-b", "main"]);
        git_in(repo.path(), &["config", "user.email", "t@example.com"]);
        git_in(repo.path(), &["config", "user.name", "T"]);
        std::fs::write(repo.path().join("f.txt"), "x").unwrap();
        git_in(repo.path(), &["add", "-A"]);
        git_in(repo.path(), &["commit", "-m", "base"]);

        // Mirrors `goal_engine::create_goal_worktree`: a linked worktree
        // living BESIDE the repo, not under it.
        let worktrees_root = tempfile::tempdir().unwrap();
        let worktree_path = worktrees_root.path().join("run-1");
        git_in(
            repo.path(),
            &[
                "worktree",
                "add",
                "-b",
                "goal/run-1",
                worktree_path.to_str().unwrap(),
            ],
        );

        let resolved = resolve_primary_checkout_from_worktree(worktree_path.to_str().unwrap())
            .await
            .expect("a real git worktree must resolve its primary checkout");
        let expected = repo.path().canonicalize().unwrap();
        let got = Path::new(&resolved).canonicalize().unwrap();
        assert_eq!(
            got, expected,
            "must recover the exact repo the worktree was created from"
        );
    }

    #[tokio::test]
    async fn resolve_primary_checkout_from_worktree_is_none_without_a_real_git_worktree() {
        assert!(
            resolve_primary_checkout_from_worktree("/no/such/permagent-d1-worktree-path")
                .await
                .is_none(),
            "a missing directory must not fabricate a checkout"
        );
        let plain = tempfile::tempdir().unwrap();
        assert!(
            resolve_primary_checkout_from_worktree(plain.path().to_str().unwrap())
                .await
                .is_none(),
            "a directory that is not a git worktree at all must not fabricate a checkout"
        );
    }
}
