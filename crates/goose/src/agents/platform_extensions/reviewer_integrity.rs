//! X-verify-1: detect a `worker_persona: "reviewer"` delegate mutating the
//! working tree despite its READ-ONLY mandate.
//!
//! F2/H1 (`summon::restrict_reviewer_tools`) narrowed the reviewer's TOOL
//! surface, but a live evidence trail (`HARNESS_LOOP_LOG.md`, Lane A2: "a
//! reviewer subagent called `edit` on `src/cache.rs` and the change shipped")
//! and Lane G's finding 2 (the reviewer keeps `shell`, which can still mutate
//! via `sed -i`, output redirection, or `rm`) both show tool restriction alone
//! is not proof. This module is the independent, mechanism-level check: it
//! does not trust the tool allowlist at all — it snapshots the actual
//! filesystem before and after the reviewer subagent runs and diffs the two.
//!
//! Deliberately NOT a revert. Destroying a subagent's output silently would
//! throw away real work (the reviewer might have made a legitimate fix a
//! human would want to keep, however out-of-mandate it was) and would hide
//! the evidence a human needs to judge that. `summon::handle_delegate` uses
//! this module's [`snapshot`]/[`diff`] only to VOID the verdict and report the
//! paths — the tree itself is left exactly as the subagent left it.

use std::collections::HashMap;
use std::path::Path;

/// A cheap, before/after comparable fingerprint of a working tree's mutable
/// state. Not a cryptographic hash — an equality check between two snapshots
/// is all this is ever used for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeSnapshot {
    /// `git status --porcelain`, parsed into path → two-letter status code.
    /// Covers tracked modifications (staged or not) and untracked files;
    /// misses changes that are made and then reverted to byte-identical
    /// content before the snapshot is taken (accepted — see module docs on
    /// the goal-dispatch `changed_lines`/`changed_paths` helpers, which take
    /// the same git-observable-state stance).
    Git(HashMap<String, String>),
    /// Git is unavailable, or `working_dir` is not a repository: mtime + size
    /// of every regular file under the tree (bounded — see
    /// [`FILE_SNAPSHOT_CAP`]), keyed by path relative to `working_dir`. A
    /// same-second rewrite to an identical size is invisible to this
    /// fallback; it exists so the detector is not a total no-op outside a
    /// git repo, not to be as precise as the git path.
    Files(HashMap<String, (u64, u64)>),
    /// Neither snapshot method produced anything usable (e.g. the directory
    /// itself could not be read). [`diff`] always reports no change against
    /// an `Unavailable` snapshot on either side — a missing snapshot is a
    /// reason to skip detection, never a reason to accuse the reviewer of a
    /// mutation it did not necessarily make.
    Unavailable,
}

/// Upper bound on how many files the mtime/size fallback will stat. A
/// reviewer's working tree is a real repository, not a case that needs
/// unbounded traversal; this keeps the fallback cheap and deterministic.
const FILE_SNAPSHOT_CAP: usize = 20_000;

/// Directory names never descended into by the mtime/size fallback — build
/// output and dependency trees are large, churn constantly for reasons that
/// have nothing to do with a reviewer's own edits, and would otherwise drown
/// out (or cap out, see [`FILE_SNAPSHOT_CAP`]) the files that matter.
const SKIPPED_DIR_NAMES: &[&str] = &[".git", "target", "node_modules", ".venv", "__pycache__"];

/// Snapshot `working_dir`'s mutable state. Tries `git status --porcelain`
/// first (cheap, exact for a real repo); falls back to mtime+size only when
/// git itself is unavailable or this is not a repository.
pub async fn snapshot(working_dir: &Path) -> TreeSnapshot {
    if let Some(git) = git_porcelain_snapshot(working_dir).await {
        return TreeSnapshot::Git(git);
    }
    file_stat_snapshot(working_dir)
}

/// `git status --porcelain` parsed into path → status code. `None` when git
/// itself failed to run or `working_dir` is not (in) a repository — the
/// caller then tries the file-stat fallback.
async fn git_porcelain_snapshot(working_dir: &Path) -> Option<HashMap<String, String>> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(working_dir)
        .args(["status", "--porcelain"])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut map = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        // Porcelain v1 format: two status columns, a space, then the path (a
        // rename/copy line instead reads "R  old -> new" — the destination is
        // what changed, so that is the path recorded). `.get` (not a byte
        // index) so a too-short or non-boundary line is skipped, never a panic.
        let Some(status) = line.get(..2) else {
            continue;
        };
        let Some(rest) = line.get(3..) else {
            continue;
        };
        let rest = rest.trim();
        let path = rest.rsplit(" -> ").next().unwrap_or(rest).trim();
        if !path.is_empty() {
            map.insert(path.to_string(), status.to_string());
        }
    }
    Some(map)
}

/// mtime+size fallback, bounded to [`FILE_SNAPSHOT_CAP`] files and skipping
/// [`SKIPPED_DIR_NAMES`]. Best-effort: any single file whose metadata can't be
/// read is simply omitted rather than aborting the whole snapshot.
fn file_stat_snapshot(working_dir: &Path) -> TreeSnapshot {
    let mut map = HashMap::new();
    let mut stack = vec![working_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if map.len() >= FILE_SNAPSHOT_CAP {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if map.len() >= FILE_SNAPSHOT_CAP {
                break;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                let name = entry.file_name();
                if SKIPPED_DIR_NAMES
                    .iter()
                    .any(|skipped| name.to_str() == Some(*skipped))
                {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            let Ok(elapsed) = modified.duration_since(std::time::UNIX_EPOCH) else {
                continue;
            };
            let Some(rel) = relative_path(working_dir, &path) else {
                continue;
            };
            map.insert(rel, (elapsed.as_secs(), metadata.len()));
        }
    }
    if map.is_empty() {
        TreeSnapshot::Unavailable
    } else {
        TreeSnapshot::Files(map)
    }
}

fn relative_path(base: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(base)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}

/// The paths that differ between two snapshots — what happened while the
/// subagent ran. An empty result means "no detected change", which is also
/// what a missing/unavailable snapshot on either side produces: the absence
/// of a snapshot is a reason not to accuse, never manufactured evidence of a
/// mutation. Comparing a [`TreeSnapshot::Git`] against a
/// [`TreeSnapshot::Files`] (the working dir somehow stopped/started being a
/// repository mid-call) is treated the same conservative way — no diff.
pub fn diff(before: &TreeSnapshot, after: &TreeSnapshot) -> Vec<String> {
    let mut changed: Vec<String> = match (before, after) {
        (TreeSnapshot::Git(before), TreeSnapshot::Git(after)) => diff_maps(before, after),
        (TreeSnapshot::Files(before), TreeSnapshot::Files(after)) => diff_maps(before, after),
        _ => Vec::new(),
    };
    changed.sort();
    changed.dedup();
    changed
}

fn diff_maps<V: PartialEq>(before: &HashMap<String, V>, after: &HashMap<String, V>) -> Vec<String> {
    let mut changed = Vec::new();
    for (path, value) in after {
        if before.get(path) != Some(value) {
            changed.push(path.clone());
        }
    }
    for path in before.keys() {
        if !after.contains_key(path) {
            changed.push(path.clone());
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .current_dir(dir.path())
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        // Mirrors `after_turn::tests::scratch_git_repo`'s hygiene exactly
        // (explicit default branch, `--no-gpg-sign`) so this never depends on
        // — or is derailed by — the host's global git config.
        run(&["-c", "init.defaultBranch=main", "init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "test"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "--no-gpg-sign", "-qm", "init"]);
        dir
    }

    #[tokio::test]
    async fn a_clean_tree_snapshots_identically_before_and_after() {
        let repo = git_repo();
        let before = snapshot(repo.path()).await;
        let after = snapshot(repo.path()).await;
        assert_eq!(before, after);
        assert!(diff(&before, &after).is_empty());
    }

    #[tokio::test]
    async fn editing_a_tracked_file_is_detected() {
        let repo = git_repo();
        let before = snapshot(repo.path()).await;
        std::fs::write(repo.path().join("a.txt"), "two\n").unwrap();
        let after = snapshot(repo.path()).await;
        let changed = diff(&before, &after);
        assert_eq!(changed, vec!["a.txt".to_string()]);
    }

    #[tokio::test]
    async fn a_new_untracked_file_is_detected() {
        let repo = git_repo();
        let before = snapshot(repo.path()).await;
        std::fs::write(repo.path().join("new.txt"), "hi\n").unwrap();
        let after = snapshot(repo.path()).await;
        assert_eq!(diff(&before, &after), vec!["new.txt".to_string()]);
    }

    #[tokio::test]
    async fn deleting_a_tracked_file_is_detected() {
        let repo = git_repo();
        let before = snapshot(repo.path()).await;
        std::fs::remove_file(repo.path().join("a.txt")).unwrap();
        let after = snapshot(repo.path()).await;
        assert_eq!(diff(&before, &after), vec!["a.txt".to_string()]);
    }

    #[test]
    fn the_file_stat_fallback_detects_a_size_change_outside_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        let before = file_stat_snapshot(dir.path());
        assert!(matches!(before, TreeSnapshot::Files(_)));
        std::fs::write(dir.path().join("a.txt"), "a much longer line now\n").unwrap();
        let after = file_stat_snapshot(dir.path());
        assert_eq!(diff(&before, &after), vec!["a.txt".to_string()]);
    }

    #[test]
    fn an_untouched_file_outside_a_repo_stays_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        let before = file_stat_snapshot(dir.path());
        let after = file_stat_snapshot(dir.path());
        assert!(diff(&before, &after).is_empty());
    }

    #[test]
    fn skipped_directories_never_contribute_false_positives() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(dir.path().join("target/debug/out.bin"), "x").unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        let before = file_stat_snapshot(dir.path());
        // Simulate a build touching only `target/` between the two snapshots.
        std::fs::write(dir.path().join("target/debug/out.bin"), "changed").unwrap();
        let after = file_stat_snapshot(dir.path());
        assert!(
            diff(&before, &after).is_empty(),
            "a change confined to a skipped directory must not be reported"
        );
    }

    #[test]
    fn comparing_a_git_snapshot_against_a_files_snapshot_never_diffs() {
        // Defensive: the two variants are never compared against each other in
        // practice (both snapshots of one call always use the same method),
        // but if they ever were, the conservative "no accusation without a
        // like-for-like comparison" rule must hold.
        let git = TreeSnapshot::Git(HashMap::from([("a.txt".to_string(), "M ".to_string())]));
        let files = TreeSnapshot::Files(HashMap::from([("b.txt".to_string(), (1, 2))]));
        assert!(diff(&git, &files).is_empty());
    }

    #[test]
    fn an_unavailable_snapshot_never_diffs() {
        assert!(diff(&TreeSnapshot::Unavailable, &TreeSnapshot::Unavailable).is_empty());
    }
}
