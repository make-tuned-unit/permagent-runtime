//! Where this user keeps their code.
//!
//! # Why this module exists
//!
//! Four separate features guessed at this independently, and on 2026-08-13 all
//! four were wrong on the same machine — a Mac whose repos live in
//! `~/Documents/dev` rather than `~/dev`:
//!
//! * the storage scanner reported a clean disk while 96 GB of cargo artifacts
//!   sat in `~/Documents/dev/<repo>/target`;
//! * 15 of 19 project `root_path` values pointed at a home directory that no
//!   longer existed;
//! * the Financier reported "no Picker checkout — nothing to start" while the
//!   checkout sat in `~/Documents/dev/Picker`;
//! * every one of them failed by finding NOTHING rather than by erroring, so
//!   each looked like a feature that simply had nothing to say.
//!
//! That last point is what makes a guess worse than a question. A wrong path
//! does not announce itself; it produces an empty result that is
//! indistinguishable from a genuinely clean machine, and the user has no reason
//! to suspect anything. One user's layout is not a default.
//!
//! So: one resolver, one config key, and discovery that reports what it found
//! rather than assuming. Onboarding proposes; the user confirms; everything
//! else reads the answer.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Config key holding the user's confirmed code directories, newest-wins over
/// discovery. A list, because splitting work across `~/dev` and
/// `~/Documents/dev` is ordinary.
pub const DEV_ROOTS_KEY: &str = "dev_roots";

/// Directory names people keep code under, checked beneath `$HOME` and
/// `$HOME/Documents`. Ordered by how commonly they are the primary root, since
/// discovery reports in this order.
const CONVENTIONAL: &[&str] = &[
    "dev",
    "code",
    "src",
    "projects",
    "repos",
    "workspace",
    "Developer", // Xcode's default
    "GitHub",    // GitHub Desktop's default
];

/// How deep below a candidate root a `.git` may sit and still count. Two levels
/// covers both `<root>/<repo>` and the common `<root>/<org>/<repo>`.
const REPO_PROBE_DEPTH: usize = 2;

/// The user's code directories: the confirmed config value when set, otherwise
/// whatever discovery can prove.
///
/// Callers should treat an empty result as "unknown", not as "the user has no
/// code" — the honest response is to say so, or to ask.
pub fn dev_roots() -> Vec<PathBuf> {
    if let Ok(configured) = crate::config::Config::global().get_param::<Vec<String>>(DEV_ROOTS_KEY)
    {
        let confirmed: Vec<PathBuf> = configured
            .iter()
            .map(|s| PathBuf::from(shellexpand::tilde(s).into_owned()))
            .filter(|p| p.is_dir())
            .collect();
        if !confirmed.is_empty() {
            return confirmed;
        }
    }
    discover_dev_roots(&home())
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Find directories under `home` that demonstrably contain git repositories.
///
/// Evidence-based on purpose: a directory is only proposed when a `.git` was
/// actually found inside it. Proposing `~/dev` because the name looks right,
/// on a machine where it is an empty leftover, is how a setup step teaches a
/// user to click through without reading.
pub fn discover_dev_roots(home: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();

    for base in [home.to_path_buf(), home.join("Documents")] {
        for name in CONVENTIONAL {
            let candidate = base.join(name);
            if !candidate.is_dir() {
                continue;
            }
            // Canonicalize before de-duping: on macOS `~/Documents/dev` and a
            // symlink to it are the same directory, and offering both would
            // make the user choose between identical options.
            let key = candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone());
            if seen.insert(key) && contains_repo(&candidate, REPO_PROBE_DEPTH) {
                found.push(candidate);
            }
        }
    }
    found
}

/// Does `dir` contain a git repository within `depth` levels?
fn contains_repo(dir: &Path, depth: usize) -> bool {
    if depth == 0 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // `.git` is a directory in a normal clone and a FILE inside a git
        // worktree, so test existence rather than directory-ness.
        if path.join(".git").exists() {
            return true;
        }
        if contains_repo(&path, depth - 1) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo_at(root: &Path, name: &str) {
        let p = root.join(name);
        fs::create_dir_all(p.join(".git")).unwrap();
    }

    /// The 2026-08-13 machine: repos under ~/Documents/dev, nothing at ~/dev.
    /// Every guesser missed this and reported an empty result.
    #[test]
    fn discovers_code_nested_under_documents() {
        let home = tempfile::tempdir().unwrap();
        let docs_dev = home.path().join("Documents/dev");
        fs::create_dir_all(&docs_dev).unwrap();
        repo_at(&docs_dev, "permagent-runtime");

        let roots = discover_dev_roots(home.path());
        assert_eq!(roots.len(), 1, "expected exactly Documents/dev: {roots:?}");
        assert!(roots[0].ends_with("Documents/dev"));
    }

    /// A conventionally-named directory with no repositories in it is a
    /// leftover, not a code root. Proposing it trains the user to click through
    /// the setup step without reading it.
    #[test]
    fn an_empty_conventional_directory_is_not_proposed() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join("dev")).unwrap();
        fs::create_dir_all(home.path().join("code/notes")).unwrap();

        assert!(
            discover_dev_roots(home.path()).is_empty(),
            "directories without repositories must not be proposed"
        );
    }

    /// Splitting work across two roots is ordinary, and both must be offered.
    #[test]
    fn reports_every_root_that_holds_repositories() {
        let home = tempfile::tempdir().unwrap();
        for base in ["dev", "Documents/code"] {
            let p = home.path().join(base);
            fs::create_dir_all(&p).unwrap();
            repo_at(&p, "a-repo");
        }
        let roots = discover_dev_roots(home.path());
        assert_eq!(roots.len(), 2, "both roots: {roots:?}");
    }

    /// `<root>/<org>/<repo>` is a normal layout and must still register.
    #[test]
    fn finds_repositories_one_organisation_deep() {
        let home = tempfile::tempdir().unwrap();
        let dev = home.path().join("dev");
        fs::create_dir_all(dev.join("acme")).unwrap();
        repo_at(&dev.join("acme"), "service");

        assert_eq!(discover_dev_roots(home.path()).len(), 1);
    }

    /// A git WORKTREE has `.git` as a file, not a directory.
    #[test]
    fn a_git_worktree_counts_as_a_repository() {
        let home = tempfile::tempdir().unwrap();
        let dev = home.path().join("dev/wt");
        fs::create_dir_all(&dev).unwrap();
        fs::write(dev.join(".git"), "gitdir: /elsewhere/.git/worktrees/wt").unwrap();

        assert_eq!(discover_dev_roots(home.path()).len(), 1);
    }
}
