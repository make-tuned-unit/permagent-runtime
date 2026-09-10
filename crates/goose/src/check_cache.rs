//! Per-goal check-result caching (Round E, coding-harness excellence loop,
//! lane E2a).
//!
//! G7's review of the Z2/Z-fix credit path flagged this directly (HARNESS_LOOP_LOG.md,
//! "LOW — repeated hold/park rechecks re-run the credit-path build/test
//! command"): every `maybe_hold_review` recheck re-executes the project's
//! build/test command via `evaluate_completion_check_locally`'s
//! `command_exit_zero` arm, even when nothing in the worktree changed since
//! the last recheck. F9's forensics on fx07 independently confirmed the same
//! pathology one layer up (inside a worker's own `verify` tool calls — 3
//! effectively-identical `cargo test` invocations back to back).
//!
//! This module is the shared primitive that closes both: a small, in-memory,
//! per-process cache keyed on **(goal id, exact command, worktree content
//! fingerprint)**. Any edit to the worktree changes the fingerprint, which is
//! part of the key — there is no separate invalidation step, and a stale
//! result can never be returned, because a stale key can never match a fresh
//! fingerprint. Nothing is ever shared across goals (`goal_id` is part of the
//! key) and nothing is ever "pre-cached" as a pass or fail before it has
//! actually run once for that exact content.
//!
//! The content fingerprint deliberately avoids `git write-tree` (which
//! requires staging every change into the index first — a mutation this
//! module must never perform on a worktree it does not own) in favor of `git
//! stash create`, which builds the same kind of commit object over the
//! tracked+staged+unstaged tree WITHOUT touching the index, the working
//! tree, or any stash ref (nothing is pushed onto `refs/stash`; the object is
//! simply returned and left dangling for git's own GC). Untracked-but-not-
//! ignored files are not covered by a stash object at all, so they are
//! folded in separately: every path `git ls-files --others --exclude-
//! standard` reports, sorted for determinism, each with its own file bytes,
//! hashed together with the stash/HEAD commit id.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};

/// The structured result of one check-command execution. Deliberately just
/// `passed` today: every call site this module serves today
/// (`evaluate_completion_check_locally`'s `command_exit_zero` arm,
/// `ReviewerMandate`'s trivial-skip computation) already collapses its own
/// check to a bool/Option before this cache would see it — there is nothing
/// richer to cache yet. Extend this struct, not the cache key shape, if a
/// future caller needs to keep stdout/stderr alongside the verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedCheckResult {
    pub passed: bool,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct CacheKey {
    goal_id: String,
    command: String,
    tree_hash: String,
}

type Cache = Mutex<HashMap<CacheKey, CachedCheckResult>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// I1 (harness loop, 2026-09-10): per-goal count of [`get_or_run`] cache
/// HITS this process has recorded — what the CLI's `harness_overhead` line
/// reports as `cache_hits=` (`turn_timing::AttemptContext::cache_hits`, E2c).
/// Kept separate from `cache()` itself so [`hit_count`] never has to guess a
/// count from the cache's own current CONTENTS (which only ever holds the
/// latest result per key, not a running tally of how many times it was
/// reused).
type HitCounts = Mutex<HashMap<String, usize>>;

fn hit_counts() -> &'static HitCounts {
    static COUNTS: OnceLock<HitCounts> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// How many [`get_or_run`] cache hits `goal_id` has recorded so far this
/// process. `0` for a goal that has never hit the cache — including one
/// that never called `get_or_run` at all — never fabricated, matching this
/// module's own "never claim a hit it cannot prove" discipline.
pub fn hit_count(goal_id: &str) -> usize {
    hit_counts()
        .lock()
        .ok()
        .and_then(|g| g.get(goal_id).copied())
        .unwrap_or(0)
}

async fn run_git(working_dir: &Path, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(working_dir)
        .args(args)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A content fingerprint for `working_dir`'s tracked+staged+unstaged tree
/// state (the `git stash create`-style object described in the module doc,
/// falling back to plain `HEAD` when there is nothing to stash) plus every
/// untracked, non-ignored file's path and bytes. `None` on any git failure —
/// not a repo, git not on `PATH`, a read error — because a cache that cannot
/// PROVE the content is unchanged must never claim a hit; callers must treat
/// `None` as "caching unavailable for this call", never as a wildcard key.
pub async fn content_fingerprint(working_dir: &Path) -> Option<String> {
    let stash = run_git(working_dir, &["stash", "create"]).await?;
    let tracked_component = if stash.trim().is_empty() {
        // Nothing to stash (clean tracked tree) — HEAD itself is the tracked
        // content identity.
        run_git(working_dir, &["rev-parse", "HEAD"]).await?
    } else {
        stash
    };

    let untracked_out =
        run_git(working_dir, &["ls-files", "--others", "--exclude-standard"]).await?;
    let mut paths: Vec<&str> = untracked_out
        .lines()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    paths.sort_unstable();

    let mut hasher = Sha256::new();
    hasher.update(tracked_component.trim().as_bytes());
    for p in &paths {
        // A NUL separator between fields so `("ab", "c")` and `("a", "bc")`
        // never collide.
        hasher.update(b"\0");
        hasher.update(p.as_bytes());
        hasher.update(b"\0");
        if let Ok(bytes) = tokio::fs::read(working_dir.join(p)).await {
            hasher.update(&bytes);
        }
    }
    Some(hex::encode(hasher.finalize()))
}

/// Look up a cached result for (`goal_id`, `command`, `tree_hash`). Callers
/// normally want [`get_or_run`] instead; this is exposed directly for tests
/// and for a caller that already has the fingerprint from a prior call this
/// turn and wants to avoid recomputing it.
pub fn lookup(goal_id: &str, command: &str, tree_hash: &str) -> Option<CachedCheckResult> {
    let key = CacheKey {
        goal_id: goal_id.to_string(),
        command: command.to_string(),
        tree_hash: tree_hash.to_string(),
    };
    cache().lock().ok()?.get(&key).cloned()
}

/// Store a fresh result for (`goal_id`, `command`, `tree_hash`). Overwrites
/// any prior entry for the same key — a real collision should never happen
/// (the key already includes the content fingerprint), but if the same
/// content somehow produced a different verdict on a re-run (a flaky check),
/// the newest run wins rather than panicking or silently keeping the stale
/// one.
pub fn store(goal_id: &str, command: &str, tree_hash: &str, result: CachedCheckResult) {
    let key = CacheKey {
        goal_id: goal_id.to_string(),
        command: command.to_string(),
        tree_hash: tree_hash.to_string(),
    };
    if let Ok(mut guard) = cache().lock() {
        guard.insert(key, result);
    }
}

/// Drop every cached entry for `goal_id`. Not required for correctness (the
/// tree-hash key already invalidates on any edit, and a goal's own id is
/// never reused) — provided so a caller that wants to bound memory once a
/// goal reaches a terminal state can do so, and so tests can isolate
/// themselves from the shared process-global cache.
pub fn clear_goal(goal_id: &str) {
    if let Ok(mut guard) = cache().lock() {
        guard.retain(|k, _| k.goal_id != goal_id);
    }
    if let Ok(mut counts) = hit_counts().lock() {
        counts.remove(goal_id);
    }
}

/// Run `check` (the actual check-command execution) unless an identical
/// check already ran, unedited, for this goal — reuse that structured result
/// instead of re-running. `command` should be the EXACT invocation text (the
/// same string a receipt/log line would show), never a normalized or
/// best-effort approximation, so two textually different but semantically
/// equivalent commands are never conflated into one cache entry.
///
/// Returns `(result, was_cache_hit)`. On a hit, logs the receipt line E2a's
/// brief asks for: `check cache hit: <cmd> (<hash>)`.
///
/// When `working_dir`'s content cannot be fingerprinted (see
/// [`content_fingerprint`]), caching is disabled for this call: `check`
/// always runs, and its result is never stored, since there would be no
/// reliable key to invalidate it under later. This is fail-open on the side
/// of RUNNING the check, never on the side of trusting a result the module
/// cannot prove is still valid.
pub async fn get_or_run<F, Fut>(
    goal_id: &str,
    command: &str,
    working_dir: &Path,
    check: F,
) -> (CachedCheckResult, bool)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = CachedCheckResult>,
{
    match content_fingerprint(working_dir).await {
        Some(hash) => {
            if let Some(cached) = lookup(goal_id, command, &hash) {
                if let Ok(mut counts) = hit_counts().lock() {
                    *counts.entry(goal_id.to_string()).or_insert(0) += 1;
                }
                tracing::info!(
                    target: "permagentd::brain",
                    goal_id = %goal_id,
                    "check cache hit: {command} ({hash})"
                );
                return (cached, true);
            }
            let result = check().await;
            store(goal_id, command, &hash, result.clone());
            (result, false)
        }
        None => (check().await, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn scratch_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success());
        };
        git(&["-c", "init.defaultBranch=main", "init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "Test"]);
        std::fs::write(dir.path().join("lib.rs"), "fn main() {}\n").unwrap();
        git(&["add", "lib.rs"]);
        git(&["commit", "--no-gpg-sign", "-qm", "init"]);
        dir
    }

    fn counting_check(
        counter: &Arc<AtomicUsize>,
        passed: bool,
    ) -> impl FnOnce() -> std::future::Ready<CachedCheckResult> {
        let counter = counter.clone();
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
            std::future::ready(CachedCheckResult { passed })
        }
    }

    // ── content_fingerprint itself ──────────────────────────────────────

    #[tokio::test]
    async fn fingerprint_is_stable_across_repeated_calls_with_no_edit() {
        let repo = scratch_git_repo();
        let a = content_fingerprint(repo.path()).await.unwrap();
        let b = content_fingerprint(repo.path()).await.unwrap();
        assert_eq!(a, b, "no edit happened — the fingerprint must not move");
    }

    #[tokio::test]
    async fn fingerprint_changes_when_a_tracked_file_is_edited() {
        let repo = scratch_git_repo();
        let before = content_fingerprint(repo.path()).await.unwrap();
        std::fs::write(repo.path().join("lib.rs"), "fn main() { /* edited */ }\n").unwrap();
        let after = content_fingerprint(repo.path()).await.unwrap();
        assert_ne!(before, after);
    }

    #[tokio::test]
    async fn fingerprint_changes_when_an_untracked_file_is_added() {
        let repo = scratch_git_repo();
        let before = content_fingerprint(repo.path()).await.unwrap();
        std::fs::write(repo.path().join("scratch.txt"), "new untracked content\n").unwrap();
        let after = content_fingerprint(repo.path()).await.unwrap();
        assert_ne!(before, after);
    }

    #[tokio::test]
    async fn fingerprint_ignores_gitignored_untracked_files() {
        let repo = scratch_git_repo();
        std::fs::write(repo.path().join(".gitignore"), "ignored.txt\n").unwrap();
        std::process::Command::new("git")
            .args(["add", ".gitignore"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--no-gpg-sign", "-qm", "add gitignore"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        let before = content_fingerprint(repo.path()).await.unwrap();
        std::fs::write(
            repo.path().join("ignored.txt"),
            "must not affect the hash\n",
        )
        .unwrap();
        let after = content_fingerprint(repo.path()).await.unwrap();
        assert_eq!(
            before, after,
            "a gitignored untracked file must never move the fingerprint"
        );
    }

    #[tokio::test]
    async fn fingerprint_of_a_non_repo_directory_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(content_fingerprint(dir.path()).await.is_none());
    }

    #[tokio::test]
    async fn fingerprint_does_not_mutate_the_working_tree_or_index() {
        let repo = scratch_git_repo();
        std::fs::write(repo.path().join("lib.rs"), "fn main() { /* dirty */ }\n").unwrap();
        let status_before = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        let _ = content_fingerprint(repo.path()).await.unwrap();
        let status_after = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert_eq!(
            status_before.stdout, status_after.stdout,
            "computing a fingerprint must never change git status (no stash push, no add)"
        );
        // The stash ref itself must stay empty — `stash create` must never
        // have been followed by a `stash store`.
        let stash_list = std::process::Command::new("git")
            .args(["stash", "list"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(
            stash_list.stdout.is_empty(),
            "no stash entry may be created"
        );
    }

    // ── get_or_run: the brief's three pinning tests ─────────────────────

    /// Same command twice with no edit in between → one execution, and the
    /// second call reports a cache hit.
    #[tokio::test]
    async fn same_command_twice_with_no_edit_runs_once() {
        let repo = scratch_git_repo();
        let goal_id = "goal-same-cmd-no-edit";
        clear_goal(goal_id);
        let counter = Arc::new(AtomicUsize::new(0));

        let (first, first_hit) = get_or_run(
            goal_id,
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;
        let (second, second_hit) = get_or_run(
            goal_id,
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "the check must run only once"
        );
        assert!(!first_hit, "the first call has nothing to hit yet");
        assert!(second_hit, "the second call must be served from the cache");
        assert_eq!(first, second);
        assert!(first.passed);
    }

    /// I1 pin (harness loop, 2026-09-10): [`hit_count`] tallies exactly the
    /// HITS a goal recorded (never the misses, never a different goal's
    /// hits), and [`clear_goal`] resets it back to zero.
    #[tokio::test]
    async fn hit_count_tallies_only_this_goals_hits() {
        let repo = scratch_git_repo();
        let goal_id = "goal-hit-count";
        let other_goal_id = "goal-hit-count-other";
        clear_goal(goal_id);
        clear_goal(other_goal_id);
        let counter = Arc::new(AtomicUsize::new(0));

        assert_eq!(hit_count(goal_id), 0, "a goal that never ran has 0 hits");

        let (_, first_hit) = get_or_run(
            goal_id,
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;
        assert!(!first_hit);
        assert_eq!(hit_count(goal_id), 0, "the first (miss) call is not a hit");

        for _ in 0..2 {
            let (_, hit) = get_or_run(
                goal_id,
                "cargo test",
                repo.path(),
                counting_check(&counter, true),
            )
            .await;
            assert!(hit);
        }
        assert_eq!(hit_count(goal_id), 2);
        assert_eq!(
            hit_count(other_goal_id),
            0,
            "a different goal's count must never be shared"
        );

        clear_goal(goal_id);
        assert_eq!(hit_count(goal_id), 0, "clear_goal resets the tally too");
    }

    /// An edit to the worktree between two identical calls → two executions
    /// (the fingerprint moved, so the second call cannot be a hit).
    #[tokio::test]
    async fn edit_between_calls_runs_twice() {
        let repo = scratch_git_repo();
        let goal_id = "goal-edit-between";
        clear_goal(goal_id);
        let counter = Arc::new(AtomicUsize::new(0));

        let (_, first_hit) = get_or_run(
            goal_id,
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;
        std::fs::write(repo.path().join("lib.rs"), "fn main() { /* changed */ }\n").unwrap();
        let (_, second_hit) = get_or_run(
            goal_id,
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            2,
            "an edit between calls must force a second real execution"
        );
        assert!(!first_hit);
        assert!(!second_hit, "the edit must invalidate the cache entry");
    }

    /// The identical command against identical, unedited content, but a
    /// different goal — never shares a cache entry.
    #[tokio::test]
    async fn different_goal_never_shares_a_cache_entry() {
        let repo = scratch_git_repo();
        clear_goal("goal-a-distinct");
        clear_goal("goal-b-distinct");
        let counter = Arc::new(AtomicUsize::new(0));

        let (_, hit_a) = get_or_run(
            "goal-a-distinct",
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;
        let (_, hit_b) = get_or_run(
            "goal-b-distinct",
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            2,
            "two different goals must never share a cache entry, even for the \
             same command over identical content"
        );
        assert!(!hit_a);
        assert!(!hit_b);
    }

    /// A failing result is cached exactly like a passing one for the SAME
    /// content (a deterministic re-run would report the same failure — this
    /// is not "caching a failure across an edit", which never happens
    /// because an edit always changes the key).
    #[tokio::test]
    async fn a_failing_result_is_also_served_from_cache_for_unedited_content() {
        let repo = scratch_git_repo();
        let goal_id = "goal-failing-result";
        clear_goal(goal_id);
        let counter = Arc::new(AtomicUsize::new(0));

        let (first, _) = get_or_run(
            goal_id,
            "cargo test",
            repo.path(),
            counting_check(&counter, false),
        )
        .await;
        let (second, second_hit) = get_or_run(
            goal_id,
            "cargo test",
            repo.path(),
            counting_check(&counter, true), // would flip the verdict if it ran
        )
        .await;

        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(second_hit);
        assert!(!first.passed);
        assert!(
            !second.passed,
            "the cached (failing) result must be reused verbatim"
        );
    }

    /// Two different commands against the same content and goal are two
    /// independent cache entries.
    #[tokio::test]
    async fn different_commands_are_independent_cache_entries() {
        let repo = scratch_git_repo();
        let goal_id = "goal-different-commands";
        clear_goal(goal_id);
        let counter = Arc::new(AtomicUsize::new(0));

        let (_, hit_a) = get_or_run(
            goal_id,
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;
        let (_, hit_b) = get_or_run(
            goal_id,
            "cargo clippy",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;

        assert_eq!(counter.load(Ordering::SeqCst), 2);
        assert!(!hit_a);
        assert!(!hit_b);
    }

    /// When the working tree cannot be fingerprinted (not a git repo),
    /// caching is disabled outright: the check always runs, never cached.
    #[tokio::test]
    async fn uncacheable_directory_always_runs_the_check() {
        let dir = tempfile::tempdir().unwrap();
        let goal_id = "goal-uncacheable";
        clear_goal(goal_id);
        let counter = Arc::new(AtomicUsize::new(0));

        let (_, first_hit) = get_or_run(
            goal_id,
            "cargo test",
            dir.path(),
            counting_check(&counter, true),
        )
        .await;
        let (_, second_hit) = get_or_run(
            goal_id,
            "cargo test",
            dir.path(),
            counting_check(&counter, true),
        )
        .await;

        assert_eq!(counter.load(Ordering::SeqCst), 2);
        assert!(!first_hit);
        assert!(!second_hit);
    }

    /// `clear_goal` drops every entry for that goal and no other.
    #[tokio::test]
    async fn clear_goal_only_clears_the_named_goal() {
        let repo = scratch_git_repo();
        clear_goal("goal-clear-x");
        clear_goal("goal-clear-y");
        let counter = Arc::new(AtomicUsize::new(0));

        get_or_run(
            "goal-clear-x",
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;
        get_or_run(
            "goal-clear-y",
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;

        clear_goal("goal-clear-x");

        let (_, hit_x) = get_or_run(
            "goal-clear-x",
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;
        let (_, hit_y) = get_or_run(
            "goal-clear-y",
            "cargo test",
            repo.path(),
            counting_check(&counter, true),
        )
        .await;

        assert!(!hit_x, "goal-clear-x's entry must have been dropped");
        assert!(hit_y, "goal-clear-y's entry must have survived");
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
