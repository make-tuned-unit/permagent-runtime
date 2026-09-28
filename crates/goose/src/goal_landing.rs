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
//! `land` itself stays fast-forward only. Lane R63b adds one headless-only
//! retry in `decisions_effects::land_approved_goal`: with parallel sibling
//! goals, a scope-disjoint branch is rebased onto the moved trunk in its own
//! worktree, re-verified, and then handed back to this same `land`.
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

    /// Whether the fix actually reached the project's trunk
    /// (`FastForwarded`/`AlreadyLanded`, `"landed_on_trunk"`) versus merely
    /// existing off-trunk — an unmerged goal branch, an uncommitted
    /// working-tree diff, or nothing at all (`NeedsMerge`/`NoCommits`/
    /// `Skipped`, `"left_in_working_tree"`).
    ///
    /// E1 (harness loop, Round E, from Lane F9 forensics 2026-09-10): F9
    /// found fx02/fx03 graded "solved" by the eval driver's own
    /// grading-time file overlay while `main`'s HEAD never moved — this
    /// in-band field lets the landing receipt distinguish a genuine landed
    /// outcome from one that stayed in the working tree, independent of
    /// (and never trusting) an external driver's own verdict. This does not
    /// change fx02/fx03's own history: their "solved" grade never called
    /// `land_approved_goal` at all (no `approve_review` decision ever
    /// fired), so this field was never written for them either — it guards
    /// every run that DOES go through this path from here on.
    pub fn trunk_state(&self) -> &'static str {
        if self.is_landed() {
            "landed_on_trunk"
        } else {
            "left_in_working_tree"
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

pub(crate) async fn git(root: &str, args: &[&str]) -> Option<(bool, String)> {
    let mut command = tokio::process::Command::new("git");
    command.kill_on_drop(true);
    let output = command
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .output();
    let out = tokio::time::timeout(std::time::Duration::from_secs(10), output)
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some((out.status.success(), text))
}

/// H1 (G8 review, harness loop Lane E3 2026-09-10):
/// [`resolve_primary_checkout_from_worktree`] recovers WHATEVER repo a
/// goal's worktree happens to be a linked worktree of — correct as far as
/// it goes ("git says this is where the worktree came from"), but that is
/// not the same claim as "this is the project the user intended". Before
/// this lane, `dispatch_goal_fn`'s own working-dir resolution fell back to
/// the daemon's `std::env::current_dir()` whenever `project.root_path` was
/// unset (`orchestrator.rs`, closed alongside this fix) — a worktree
/// created under that fallback branches off whatever directory the process
/// happened to be started from, which in the reported scenario was the
/// `permagent-runtime` checkout itself. `land_approved_goal`'s own doc
/// comment states the invariant that chain violates: "the daemon's cwd is
/// emphatically not it". A stale card from before that fix shipped, or any
/// future regression that reintroduces the same mistake, would still leave
/// this worktree-derived fallback trusting whatever it recovers.
///
/// So the fallback is trusted only when BOTH of the following hold — G8's
/// own recommendation was to "compare against `card.metadata_json`'s own
/// recorded working_dir" rather than trust `git-common-dir` alone:
///
/// 1. **Known location** — `candidate_root` must match a working directory
///    independently recorded elsewhere for this goal: its dispatching
///    session's own `sessions.working_dir` (never re-derived from the
///    process's live `current_dir()`, which is exactly the untrusted value
///    that created this hazard). Without a positive match here, "git says
///    this is where it came from" alone is not enough.
/// 2. **Ancestry** — the goal's own dispatch-time baseline
///    (`dispatch_evidence.baseline_commit`, recorded by the orchestrator
///    before the worker ever touched the worktree) must be an ancestor of
///    the candidate root's current HEAD. A repo whose history never
///    contained the goal's own recorded starting point cannot be the repo
///    the goal was dispatched against, regardless of what its worktree
///    metadata claims.
///
/// A `None` input (no baseline recorded, or no known working dir to compare
/// against) refuses rather than assuming innocence — an unverifiable
/// fallback landing is exactly the silent hazard this closes.
pub(crate) async fn verify_worktree_landing_target(
    candidate_root: &str,
    baseline_commit: Option<&str>,
    known_working_dir: Option<&str>,
) -> Result<(), String> {
    let Some(known) = known_working_dir.filter(|s| !s.is_empty()) else {
        return Err(
            "no independently recorded working directory exists for this goal (its \
             dispatching session has none on record) — an unverified worktree-derived \
             fallback target is never trusted on its own say-so"
                .to_string(),
        );
    };
    if !paths_match(known, candidate_root) {
        return Err(format!(
            "recovered '{candidate_root}' via the worktree's own git metadata, but the \
             goal's recorded working directory is '{known}' — refusing a mismatched \
             fallback target"
        ));
    }

    let Some(baseline) = baseline_commit.filter(|s| !s.is_empty()) else {
        return Err(
            "the goal has no recorded dispatch baseline commit to verify the landing \
             target's history against"
                .to_string(),
        );
    };
    match git(
        candidate_root,
        &["merge-base", "--is-ancestor", baseline, "HEAD"],
    )
    .await
    {
        Some((true, _)) => Ok(()),
        _ => Err(format!(
            "'{candidate_root}' does not contain the goal's own dispatch baseline \
             ({baseline}) as an ancestor of its HEAD"
        )),
    }
}

/// Loose path equality for comparing a recorded working directory against a
/// recovered one — tolerates a trailing-slash difference via a cheap raw
/// comparison first.
///
/// I1 (harness loop, 2026-09-10, gate fix): the raw comparison alone missed
/// the common case where one side is git's own canonicalized absolute path
/// (`resolve_primary_checkout_from_worktree` uses `git rev-parse --path-
/// format=absolute`, which resolves through OS symlinks — e.g. macOS's
/// `/var` -> `/private/var`) and the other is the RAW path a session
/// recorded its `working_dir` as, un-resolved. Two paths naming the exact
/// same real directory then compared unequal, refusing a landing that
/// should have been trusted. Falls back to canonicalizing both sides only
/// when the raw comparison fails; if either side cannot be resolved
/// (deleted, permissions), that failure is itself treated as "no match" —
/// never as a reason to assume innocence.
///
/// Lane R22f: `pub(crate)` (was private) so `orchestrator.rs`'s salvage
/// path can reuse this exact comparison to refuse a resolved landing root
/// that turns out to BE the goal's own worktree (a degenerate shape every
/// non-linked-worktree test fixture hits) — see `resolve_trusted_landing_
/// root`'s own doc comment.
pub(crate) fn paths_match(a: &str, b: &str) -> bool {
    fn norm(s: &str) -> &str {
        s.trim_end_matches('/')
    }
    if norm(a) == norm(b) {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// Fast-forward the project's trunk onto an approved goal's work.
/// Cross-process serialization for cooperating Permagent landing paths. Git's
/// own ref/index locks still protect the mutation; external Git users do not
/// participate in this advisory lock and can cause an honest landing refusal.
pub(crate) struct LandingGuard {
    _file: std::fs::File,
    root: std::path::PathBuf,
}

pub(crate) async fn acquire_landing_guard(root: &str) -> Result<LandingGuard, String> {
    use fs2::FileExt;
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let (ok, common) = git(
        root.to_str().ok_or("non-UTF8 repository root")?,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await
    .ok_or("cannot resolve Git common directory")?;
    if !ok {
        return Err("cannot resolve Git common directory".into());
    }
    let common = std::fs::canonicalize(common).map_err(|e| e.to_string())?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(common.join("permagent-landing.lock"))
        .map_err(|e| e.to_string())?;
    file.try_lock_exclusive()
        .map_err(|e| format!("another landing owns the repository lock: {e}"))?;
    Ok(LandingGuard { _file: file, root })
}

pub async fn land(req: &LandRequest) -> LandOutcome {
    let guard = match acquire_landing_guard(&req.repo_root).await {
        Ok(guard) => guard,
        Err(reason) => return LandOutcome::Skipped(reason),
    };
    land_under_guard(req, &guard).await
}

pub(crate) async fn land_under_guard(req: &LandRequest, guard: &LandingGuard) -> LandOutcome {
    if std::fs::canonicalize(&req.repo_root).ok().as_ref() != Some(&guard.root) {
        return LandOutcome::Skipped("landing lock does not match the repository root".into());
    }
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

    /// E1 (harness loop, Round E, from Lane F9 forensics 2026-09-10): the
    /// landing receipt must distinguish "the fix actually reached trunk"
    /// from every shape where it did not, independent of `outcome_key`'s own
    /// coarser "landed"/"needs_merge"/"skipped" grouping (which already
    /// folds `NeedsMerge` and `Skipped`/`NoCommits` together as "not
    /// landed" but does not say so in a field named for exactly that).
    /// Pre-fix: `trunk_state` does not exist at all (fails to compile).
    #[test]
    fn trunk_state_distinguishes_landed_from_everything_left_off_trunk() {
        assert_eq!(
            LandOutcome::FastForwarded {
                branch: "goal/cli-1".into(),
                trunk: "main".into(),
                head: "deadbeefcafe".into(),
            }
            .trunk_state(),
            "landed_on_trunk"
        );
        assert_eq!(
            LandOutcome::AlreadyLanded {
                trunk: "main".into(),
            }
            .trunk_state(),
            "landed_on_trunk"
        );
        assert_eq!(
            LandOutcome::NeedsMerge {
                branch: "goal/cli-1".into(),
                trunk: "main".into(),
            }
            .trunk_state(),
            "left_in_working_tree",
            "a real commit stuck on an unmerged branch is not on trunk"
        );
        assert_eq!(LandOutcome::NoCommits.trunk_state(), "left_in_working_tree");
        assert_eq!(
            LandOutcome::Skipped("no root_path".into()).trunk_state(),
            "left_in_working_tree"
        );
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

    /// A repo with a "base" commit and a `goal/run-1` branch one commit
    /// ahead, mirroring `repo_with_finished_goal_branch` in
    /// `decisions_effects.rs`. Returns (repo, base commit sha, branch head
    /// sha).
    ///
    /// I1 (harness loop, 2026-09-10, gate fix): the base commit's content
    /// includes a fresh UUID so two calls to this helper (as
    /// `verify_worktree_landing_target_refuses_when_baseline_is_not_an_
    /// ancestor` makes, for two UNRELATED repos) can never produce the SAME
    /// commit sha — a real git footgun this helper originally hit: with
    /// identical tree content, message, author and committer, and no
    /// microsecond-resolution timestamp guarantee, two back-to-back first
    /// commits in fresh repos can hash identically, silently making
    /// "unrelated" baselines actually equal.
    fn repo_with_base_and_goal_branch() -> (tempfile::TempDir, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git_in(root, &["init", "-b", "main"]);
        git_in(root, &["config", "user.email", "t@example.com"]);
        git_in(root, &["config", "user.name", "T"]);
        std::fs::write(
            root.join("README.md"),
            format!("base {}\n", uuid::Uuid::new_v4()),
        )
        .unwrap();
        git_in(root, &["add", "-A"]);
        git_in(root, &["commit", "-m", "base"]);
        let base = git_in(root, &["rev-parse", "HEAD"]);
        git_in(root, &["checkout", "-b", "goal/run-1"]);
        std::fs::write(root.join("work.txt"), "work\n").unwrap();
        git_in(root, &["add", "-A"]);
        git_in(root, &["commit", "-m", "the work"]);
        let head = git_in(root, &["rev-parse", "HEAD"]);
        git_in(root, &["checkout", "main"]);
        (dir, base, head)
    }

    /// H1 (G8 review, Lane E3 2026-09-10): without a positive match against
    /// an independently recorded working directory, the fallback must
    /// refuse outright — "git resolved SOME directory" is not enough on its
    /// own. Pre-fix: this check does not exist at all.
    #[tokio::test]
    async fn verify_worktree_landing_target_refuses_without_a_known_working_dir() {
        let (repo, base, _head) = repo_with_base_and_goal_branch();
        let root = repo.path().to_string_lossy().into_owned();
        let err = verify_worktree_landing_target(&root, Some(&base), None)
            .await
            .expect_err("no known working dir must refuse, not assume innocence");
        assert!(
            err.contains("no independently recorded working directory"),
            "{err}"
        );
    }

    /// The exact H1 hazard: the worktree's own git metadata resolves to
    /// SOME real repo (here, a stand-in for the daemon's own
    /// `permagent-runtime` checkout, repo B), but the goal's independently
    /// recorded working directory is a DIFFERENT repo (repo A) — refuse,
    /// never fast-forward the wrong one.
    #[tokio::test]
    async fn verify_worktree_landing_target_refuses_when_known_working_dir_does_not_match() {
        let (repo_a, base_a, _head_a) = repo_with_base_and_goal_branch();
        let (repo_b, _base_b, _head_b) = repo_with_base_and_goal_branch();
        let root_b = repo_b.path().to_string_lossy().into_owned();
        let known_a = repo_a.path().to_string_lossy().into_owned();

        let err = verify_worktree_landing_target(&root_b, Some(&base_a), Some(&known_a))
            .await
            .expect_err("a mismatched known working dir must refuse the fallback target");
        assert!(
            err.contains("refusing a mismatched fallback target"),
            "{err}"
        );
    }

    /// A goal with no recorded dispatch baseline cannot have its history
    /// verified at all — refuse rather than skip the check.
    #[tokio::test]
    async fn verify_worktree_landing_target_refuses_without_a_baseline_commit() {
        let (repo, _base, _head) = repo_with_base_and_goal_branch();
        let root = repo.path().to_string_lossy().into_owned();
        let err = verify_worktree_landing_target(&root, None, Some(&root))
            .await
            .expect_err("no recorded baseline must refuse an unverifiable landing");
        assert!(err.contains("no recorded dispatch baseline"), "{err}");
    }

    /// A baseline that is not actually an ancestor of the candidate's HEAD
    /// (e.g. from a wholly different repo's history) must refuse — the
    /// candidate's history does not contain the goal's own starting point.
    #[tokio::test]
    async fn verify_worktree_landing_target_refuses_when_baseline_is_not_an_ancestor() {
        let (repo_a, _base_a, _head_a) = repo_with_base_and_goal_branch();
        let (_repo_b, base_b, _head_b) = repo_with_base_and_goal_branch();
        let root_a = repo_a.path().to_string_lossy().into_owned();

        let err = verify_worktree_landing_target(&root_a, Some(&base_b), Some(&root_a))
            .await
            .expect_err("an unrelated baseline must fail the ancestry check");
        assert!(
            err.contains("does not contain the goal's own dispatch baseline"),
            "{err}"
        );
    }

    /// The positive case (G8's pinning test 2 — "same setup with the
    /// session's project root = repo A → lands on A"): a known working dir
    /// that matches the candidate, and a baseline that really is an
    /// ancestor of its HEAD, is accepted.
    #[tokio::test]
    async fn verify_worktree_landing_target_succeeds_when_both_checks_pass() {
        let (repo, base, _head) = repo_with_base_and_goal_branch();
        let root = repo.path().to_string_lossy().into_owned();
        verify_worktree_landing_target(&root, Some(&base), Some(&root))
            .await
            .expect("a matching known working dir plus real ancestry must be accepted");
    }
    #[tokio::test]
    async fn concurrent_landing_lock_refuses_before_git_and_releases_on_drop() {
        let (repo, _base, head) = repo_with_base_and_goal_branch();
        let root = repo.path().to_string_lossy().into_owned();
        let before = git_in(repo.path(), &["rev-parse", "HEAD"]);
        let guard = acquire_landing_guard(&root).await.unwrap();
        let outcome = land(&LandRequest {
            repo_root: root.clone(),
            branch: "goal/test".into(),
            head_commit: head,
        })
        .await;
        assert!(
            matches!(outcome, LandOutcome::Skipped(ref reason) if reason.contains("repository lock"))
        );
        assert_eq!(git_in(repo.path(), &["rev-parse", "HEAD"]), before);
        assert!(acquire_landing_guard(&root).await.is_err());
        drop(guard);
        assert!(acquire_landing_guard(&root).await.is_ok());
    }
}
