//! Repo-health surveying for the Steward's git-health lane: PURE parsers over
//! git's stable porcelain outputs, the reapable/deletable predicates, and the
//! read-only collector the sweep loop drives.
//!
//! Nothing in this module mutates a repository. Detections become either a
//! Tier-2 proposal (via `hygiene::propose_repo_hygiene`) or an alert-only
//! surface — decided by the caller, never here.

use std::path::Path;

use super::hygiene::{branch_is_merged, git_checked, has_unpushed_work};
use super::is_protected_branch;

/// One record from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    /// Absolute path of the worktree.
    pub path: String,
    /// HEAD commit sha, absent for a bare entry.
    pub head: Option<String>,
    /// Short branch name (`refs/heads/` stripped); `None` when detached/bare.
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
}

/// Parse `git worktree list --porcelain`. Records are attribute lines
/// (`worktree <path>`, `HEAD <sha>`, `branch <ref>`, bare-word `detached` /
/// `bare`) separated by blank lines; the final record may lack its trailing
/// blank line.
pub fn parse_worktree_list_porcelain(s: &str) -> Vec<WorktreeEntry> {
    let mut out = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    for line in s.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            if let Some(e) = current.take() {
                out.push(e);
            }
            continue;
        }
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(e) = current.take() {
                out.push(e);
            }
            current = Some(WorktreeEntry {
                path: path.to_string(),
                head: None,
                branch: None,
                detached: false,
                bare: false,
            });
            continue;
        }
        let Some(e) = current.as_mut() else { continue };
        if let Some(sha) = line.strip_prefix("HEAD ") {
            e.head = Some(sha.to_string());
        } else if let Some(r) = line.strip_prefix("branch ") {
            e.branch = Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string());
        } else if line == "detached" {
            e.detached = true;
        } else if line == "bare" {
            e.bare = true;
        }
    }
    if let Some(e) = current.take() {
        out.push(e);
    }
    out
}

/// One local branch as listed by
/// `git for-each-ref --format='%(refname:short)%09%(objectname:short)%09%(committerdate:iso8601-strict)' refs/heads`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRef {
    pub name: String,
    pub sha: String,
    /// ISO-8601 committer date — lexicographically sortable (oldest first).
    pub committer_date: String,
}

/// The exact format string the collector passes to `for-each-ref`, kept beside
/// its parser so they cannot drift apart.
pub const FOR_EACH_REF_FORMAT: &str =
    "%(refname:short)%09%(objectname:short)%09%(committerdate:iso8601-strict)";

/// Parse the tab-separated `for-each-ref` output produced with
/// [`FOR_EACH_REF_FORMAT`]. Malformed lines are skipped, never guessed at.
pub fn parse_for_each_ref(s: &str) -> Vec<BranchRef> {
    s.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?.trim();
            let sha = parts.next()?.trim();
            let date = parts.next().unwrap_or("").trim();
            if name.is_empty() || sha.is_empty() {
                return None;
            }
            Some(BranchRef {
                name: name.to_string(),
                sha: sha.to_string(),
                committer_date: date.to_string(),
            })
        })
        .collect()
}

/// A surveyed non-primary worktree with its safety facts resolved.
#[derive(Debug, Clone)]
pub struct WorktreeStatus {
    pub entry: WorktreeEntry,
    /// Branch merged into the trunk. Detached worktrees resolve `false` here —
    /// there is no branch to prove merged, so the sweep never proposes them
    /// (the on-transition goal reaper owns detached goal worktrees).
    pub merged: bool,
    /// How merged-ness was decided, for the evidence line.
    pub merged_via: Option<String>,
    /// `git status --porcelain` empty. Unreadable resolves `false` (not clean).
    pub clean: bool,
    /// Commits not on any remote. Unprovable resolves `true` (assume unpushed).
    pub unpushed: bool,
}

/// A worktree is safe to PROPOSE reaping only when merged AND clean AND fully
/// pushed. Every one of these is re-verified at effect time regardless.
pub fn reapable(w: &WorktreeStatus) -> bool {
    w.merged && w.clean && !w.unpushed
}

/// A surveyed local branch with its safety facts resolved.
#[derive(Debug, Clone)]
pub struct BranchStatus {
    pub branch: BranchRef,
    pub merged: bool,
    pub merged_via: Option<String>,
    /// Checked out in any worktree (including the primary).
    pub checked_out: bool,
}

/// A branch is safe to PROPOSE deleting only when merged, not protected, and
/// not checked out anywhere. Re-verified at effect time regardless.
pub fn deletable(b: &BranchStatus) -> bool {
    b.merged && !is_protected_branch(&b.branch.name) && !b.checked_out
}

/// Bound on merged-ness checks per pass: each can cost a `gh` round-trip, and
/// a repo with hundreds of stale branches must not turn one sweep into a
/// hammering session. Oldest branches go first; the rest wait for later passes.
const MAX_MERGE_CHECKS_PER_PASS: usize = 12;

/// Read-only survey of one repository.
#[derive(Debug, Clone)]
pub struct RepoHealth {
    pub repo_path: String,
    /// Non-primary, non-bare worktrees with their safety facts.
    pub worktrees: Vec<WorktreeStatus>,
    /// Local branches surveyed this pass (bounded — not necessarily all).
    pub branches: Vec<BranchStatus>,
    /// The PRIMARY working tree has uncommitted changes (alert-only signal).
    pub dirty_primary: bool,
}

/// Survey `repo` read-only. `None` when the path is not a readable git repo —
/// a stated fact for the caller to log, never a pretend-empty report.
pub async fn collect_repo_health(repo: &Path) -> Option<RepoHealth> {
    git_checked(repo, &["rev-parse", "--git-dir"]).await?;
    // Fresh refs for merged/pushed checks; failure tolerated (stale refs err
    // toward "not merged"/"unpushed", i.e. toward proposing nothing).
    if git_checked(repo, &["fetch", "--prune", "origin"])
        .await
        .is_none()
    {
        tracing::warn!(
            target: "steward",
            repo = %repo.display(),
            "fetch --prune failed — surveying on possibly-stale refs"
        );
    }

    let listing = git_checked(repo, &["worktree", "list", "--porcelain"]).await?;
    let entries = parse_worktree_list_porcelain(&listing);
    let repo_canon = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());

    let mut checks_left = MAX_MERGE_CHECKS_PER_PASS;
    let mut worktrees = Vec::new();
    let mut checked_out_branches: Vec<String> = Vec::new();
    for entry in &entries {
        if let Some(b) = &entry.branch {
            checked_out_branches.push(b.clone());
        }
        let path = Path::new(&entry.path);
        let is_primary =
            std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()) == repo_canon;
        if entry.bare || is_primary {
            continue;
        }
        let clean = matches!(
            git_checked(path, &["status", "--porcelain"])
                .await
                .as_deref(),
            Some("")
        );
        let unpushed = has_unpushed_work(path).await.unwrap_or(true);
        let (merged, merged_via) = match &entry.branch {
            Some(branch) if checks_left > 0 => {
                checks_left -= 1;
                match branch_is_merged(repo, branch).await {
                    Some((m, via)) => (m, Some(via)),
                    None => (false, None),
                }
            }
            _ => (false, None),
        };
        worktrees.push(WorktreeStatus {
            entry: entry.clone(),
            merged,
            merged_via,
            clean,
            unpushed,
        });
    }

    let refs_out = git_checked(
        repo,
        &[
            "for-each-ref",
            &format!("--format={FOR_EACH_REF_FORMAT}"),
            "refs/heads",
        ],
    )
    .await
    .unwrap_or_default();
    let mut refs = parse_for_each_ref(&refs_out);
    refs.sort_by(|a, b| a.committer_date.cmp(&b.committer_date));
    let mut branches = Vec::new();
    for branch in refs {
        let checked_out = checked_out_branches.iter().any(|b| b == &branch.name);
        // Protected and checked-out branches can never become proposals, so
        // don't spend a bounded merge check on them.
        let skip_check = is_protected_branch(&branch.name) || checked_out || checks_left == 0;
        let (merged, merged_via) = if skip_check {
            (false, None)
        } else {
            checks_left -= 1;
            match branch_is_merged(repo, &branch.name).await {
                Some((m, via)) => (m, Some(via)),
                None => (false, None),
            }
        };
        branches.push(BranchStatus {
            branch,
            merged,
            merged_via,
            checked_out,
        });
    }

    let dirty_primary = !matches!(
        git_checked(repo, &["status", "--porcelain"])
            .await
            .as_deref(),
        Some("")
    );

    Some(RepoHealth {
        repo_path: repo.to_string_lossy().to_string(),
        worktrees,
        branches,
        dirty_primary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORCELAIN_FIXTURE: &str = "worktree /Users/j/dev/proj\n\
HEAD 1111111111111111111111111111111111111111\n\
branch refs/heads/main\n\
\n\
worktree /Users/j/dev/wt/feature-x\n\
HEAD 2222222222222222222222222222222222222222\n\
branch refs/heads/feature/x\n\
\n\
worktree /Users/j/dev/wt/detached-goal\n\
HEAD 3333333333333333333333333333333333333333\n\
detached\n\
\n\
worktree /Users/j/dev/proj-bare.git\n\
bare\n";

    #[test]
    fn porcelain_parser_handles_branch_detached_and_bare() {
        let e = parse_worktree_list_porcelain(PORCELAIN_FIXTURE);
        assert_eq!(e.len(), 4);

        assert_eq!(e[0].path, "/Users/j/dev/proj");
        assert_eq!(e[0].branch.as_deref(), Some("main"));
        assert!(!e[0].detached && !e[0].bare);

        assert_eq!(e[1].branch.as_deref(), Some("feature/x"));
        assert_eq!(
            e[1].head.as_deref(),
            Some("2222222222222222222222222222222222222222")
        );

        assert_eq!(e[2].path, "/Users/j/dev/wt/detached-goal");
        assert!(e[2].detached);
        assert!(e[2].branch.is_none());

        assert!(e[3].bare);
        assert!(e[3].head.is_none());
    }

    #[test]
    fn porcelain_parser_flushes_a_final_record_without_trailing_blank() {
        let s = "worktree /a\nHEAD 1234\nbranch refs/heads/x";
        let e = parse_worktree_list_porcelain(s);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].branch.as_deref(), Some("x"));
    }

    #[test]
    fn porcelain_parser_empty_input_is_empty() {
        assert!(parse_worktree_list_porcelain("").is_empty());
    }

    #[test]
    fn for_each_ref_parser_reads_tab_rows_and_skips_malformed() {
        let s = "main\tabc1234\t2026-08-01T10:00:00+00:00\n\
feature/x\tdef5678\t2026-07-01T10:00:00+00:00\n\
garbage-line-without-tabs\n";
        let refs = parse_for_each_ref(s);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "main");
        assert_eq!(refs[1].sha, "def5678");
        assert_eq!(refs[1].committer_date, "2026-07-01T10:00:00+00:00");
    }

    fn wt(merged: bool, clean: bool, unpushed: bool) -> WorktreeStatus {
        WorktreeStatus {
            entry: WorktreeEntry {
                path: "/tmp/wt".into(),
                head: None,
                branch: Some("feature/x".into()),
                detached: false,
                bare: false,
            },
            merged,
            merged_via: None,
            clean,
            unpushed,
        }
    }

    #[test]
    fn reapable_requires_merged_clean_and_pushed() {
        assert!(reapable(&wt(true, true, false)));
        assert!(!reapable(&wt(false, true, false)), "unmerged is kept");
        assert!(!reapable(&wt(true, false, false)), "dirty is kept");
        assert!(!reapable(&wt(true, true, true)), "unpushed is kept");
    }

    fn br(name: &str, merged: bool, checked_out: bool) -> BranchStatus {
        BranchStatus {
            branch: BranchRef {
                name: name.into(),
                sha: "abc1234".into(),
                committer_date: "2026-01-01T00:00:00+00:00".into(),
            },
            merged,
            merged_via: None,
            checked_out,
        }
    }

    #[test]
    fn deletable_requires_merged_unprotected_not_checked_out() {
        assert!(deletable(&br("feature/old", true, false)));
        assert!(!deletable(&br("feature/old", false, false)), "unmerged");
        assert!(!deletable(&br("main", true, false)), "protected");
        assert!(
            !deletable(&br("release-1.0", true, false)),
            "protected prefix"
        );
        assert!(!deletable(&br("feature/live", true, true)), "checked out");
    }
}
