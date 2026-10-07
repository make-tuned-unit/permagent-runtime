use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/");
    println!("cargo:rerun-if-changed=build.rs");
    watch_git_head();

    // Runtime dylib discovery for the daemon binaries (macOS).
    //
    // The daemon dynamically links libsherpa-onnx-c-api.dylib (STT via
    // sherpa-onnx), which in turn pulls in libonnxruntime. A plain
    // `cargo build` (debug or release) drops these dylibs into
    // `target/<profile>/` — the SAME directory as the `permagentd` binary —
    // but Cargo does not add an rpath that points there, so the binary
    // cannot find them at runtime without DYLD_FALLBACK_LIBRARY_PATH.
    // Debug builds run in-place from worktrees hit exactly this (issue #295);
    // the bundled app dodges it because ui/desktop/scripts/copy-sidecar.sh
    // rewrites the rpath to `@executable_path/../Frameworks` after the fact.
    //
    // Adding `@executable_path` as an rpath makes the binary look in its own
    // directory — where the dylibs already sit for in-place builds — fixing
    // debug (and raw release) runs. It is harmless for the bundle: an extra
    // rpath pointing at Contents/MacOS/ simply finds nothing and falls
    // through to the `../Frameworks` rpath copy-sidecar.sh still adds.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // Applies to every bin target in this package (permagentd et al.),
        // all of which link the same sherpa-onnx dylibs.
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path");
    }

    // Git SHA
    let sha = git(&["rev-parse", "--short=9", "HEAD"]);
    println!("cargo:rustc-env=PERMAGENT_GIT_SHA={}", sha);

    // Git branch
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]);
    println!("cargo:rustc-env=PERMAGENT_GIT_BRANCH={}", branch);

    // Build timestamp (UTC, ISO 8601)
    let ts = chrono_now_utc();
    println!("cargo:rustc-env=PERMAGENT_BUILD_TIMESTAMP={}", ts);

    // Rust version
    let rustc = rustc_version();
    println!("cargo:rustc-env=PERMAGENT_RUST_VERSION={}", rustc);

    // Git dirty flag
    let dirty = git_dirty();
    println!("cargo:rustc-env=PERMAGENT_GIT_DIRTY={}", dirty);

    // Spectral pin — extract rev from Cargo.lock
    let spectral_pin = spectral_rev();
    println!("cargo:rustc-env=PERMAGENT_SPECTRAL_PIN={}", spectral_pin);
}

/// Watch whatever actually records HEAD, so the baked SHA invalidates when the
/// checkout moves.
///
/// `../../.git` is a directory in a primary checkout but a FILE holding
/// `gitdir: <path>` in a linked worktree. Watching `../../.git/HEAD` therefore
/// watches nothing at all in a worktree — and because declaring ANY
/// `rerun-if-changed` opts this script out of Cargo's default "rerun when the
/// package changed", nothing else re-triggered it either. A release built from a
/// worktree kept reporting the SHA of whichever commit was checked out the first
/// time it was built, across every later checkout, silently. That matters
/// because the release recipe builds from a worktree, so the DMG's
/// `/api/version` could disagree with the code inside it.
fn watch_git_head() {
    let manifest_dir = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_else(|| ".".into()),
    );
    let dot_git = manifest_dir.join("../../.git");
    for path in git_watch_paths(&dot_git) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn git_watch_paths(dot_git: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Some((git_dir, common_dir)) = git_dirs(dot_git) else {
        // No git metadata (a source tarball, a vendored build). Nothing to
        // watch; the SHA falls back to "unknown" as it always has.
        return Vec::new();
    };

    let head = git_dir.join("HEAD");
    if !head.is_file() {
        return Vec::new();
    }
    let mut paths = vec![head.clone()];

    // HEAD changes on checkout even when detached. On a branch, the ref itself
    // changes on commit; in linked worktrees it lives in the shared common dir.
    // Watch only that ref, rather than the entire shared refs tree, so another
    // worktree's branch update does not rebuild this package.
    if let Some(reference) = head_ref(&head) {
        let reference_path = common_dir.join(&reference);
        if reference_path.is_file() {
            // A loose branch ref is precise: other branches cannot trigger it.
            paths.push(reference_path);
        } else {
            // A packed branch has no loose ref file. Watch packed-refs only
            // when it contains this branch. Updates to other packed refs can
            // still rerun this build script because Git stores them together.
            let packed_refs = common_dir.join("packed-refs");
            if packed_refs.is_file() && packed_refs_contains(&packed_refs, &reference) {
                paths.push(packed_refs);
            }

            // The loose ref may be created after this build. Cargo treats a
            // nonexistent rerun path as perpetually dirty. Prefer the
            // per-worktree HEAD reflog when enabled; otherwise watch the
            // nearest existing parent directory to detect ref creation.
            let head_reflog = git_dir.join("logs/HEAD");
            if head_reflog.is_file() {
                paths.push(head_reflog);
            } else if let Some(parent) = nearest_existing_parent(&reference_path) {
                paths.push(parent);
            }
        }
    }

    paths.sort();
    paths.dedup();
    paths
}

/// Resolve a checkout's private gitdir and shared common gitdir. `.git` can be
/// a directory in a primary checkout or a `gitdir:` pointer in a worktree.
fn git_dirs(dot_git: &std::path::Path) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let git_dir = if dot_git.is_file() {
        let text = std::fs::read_to_string(dot_git).ok()?;
        let target = text.lines().find_map(|line| line.strip_prefix("gitdir:"))?;
        resolve_from(dot_git.parent()?, target.trim())
    } else if dot_git.is_dir() {
        dot_git.to_path_buf()
    } else {
        return None;
    };

    let common_dir = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => resolve_from(&git_dir, text.trim()),
        Err(_) => git_dir.clone(),
    };
    Some((git_dir, common_dir))
}

fn resolve_from(base: &std::path::Path, path: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn nearest_existing_parent(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut parent = path.parent()?;
    loop {
        if parent.is_dir() {
            return Some(parent.to_path_buf());
        }
        parent = parent.parent()?;
    }
}

/// Return a safe symbolic HEAD target. Detached HEADs are tracked through the
/// per-worktree HEAD file alone. Reject malformed or escaping ref paths.
fn head_ref(head: &std::path::Path) -> Option<std::path::PathBuf> {
    let text = std::fs::read_to_string(head).ok()?;
    let reference = text.trim().strip_prefix("ref: ")?;
    let path = std::path::Path::new(reference);
    let mut components = path.components();
    if components.next()? != std::path::Component::Normal(std::ffi::OsStr::new("refs")) {
        return None;
    }
    let remaining: Vec<_> = components.collect();
    if remaining.is_empty()
        || remaining
            .iter()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_path_buf())
}

fn packed_refs_contains(packed_refs: &std::path::Path, reference: &std::path::Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(packed_refs) else {
        return false;
    };
    let Some(reference) = reference.to_str() else {
        return false;
    };
    contents.lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.next().is_some_and(|object_id| {
            matches!(object_id.len(), 40 | 64)
                && object_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                && fields.next() == Some(reference)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "goose-build-git-watch-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn linked_worktree_watcher_resolves_only_its_common_branch_ref() {
        let scratch = Scratch::new();
        let root = &scratch.0;
        let worktree = root.join("worktree");
        let git_dir = root.join("common/worktrees/wt");
        let common_dir = root.join("common");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::create_dir_all(common_dir.join("refs/heads")).unwrap();
        std::fs::write(worktree.join(".git"), "gitdir: ../common/worktrees/wt\n").unwrap();
        std::fs::write(git_dir.join("commondir"), "../..\n").unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/task-branch\n").unwrap();
        std::fs::write(
            common_dir.join("packed-refs"),
            "# pack-refs with: peeled fully-peeled\n0123456789012345678901234567890123456789 refs/heads/task-branch\n",
        )
        .unwrap();

        let (resolved_git_dir, resolved_common_dir) = git_dirs(&worktree.join(".git")).unwrap();
        assert_eq!(
            resolved_git_dir.canonicalize().unwrap(),
            git_dir.canonicalize().unwrap()
        );
        assert_eq!(
            resolved_common_dir.canonicalize().unwrap(),
            common_dir.canonicalize().unwrap()
        );
        let reference = head_ref(&resolved_git_dir.join("HEAD")).unwrap();
        assert_eq!(
            reference,
            std::path::PathBuf::from("refs/heads/task-branch")
        );
        assert!(packed_refs_contains(
            &resolved_common_dir.join("packed-refs"),
            &reference
        ));

        let watched = git_watch_paths(&worktree.join(".git"));
        assert!(watched.contains(&resolved_git_dir.join("HEAD")));
        assert!(watched.contains(&resolved_common_dir.join("packed-refs")));
        assert!(watched.contains(&resolved_common_dir.join("refs/heads")));
        assert!(watched.iter().all(|path| path.exists()));
        assert!(!watched.contains(&resolved_common_dir.join(&reference)));

        // A private HEAD reflog replaces the broader directory fallback.
        std::fs::create_dir_all(resolved_git_dir.join("logs")).unwrap();
        std::fs::write(resolved_git_dir.join("logs/HEAD"), "reflog\n").unwrap();
        let watched = git_watch_paths(&worktree.join(".git"));
        assert!(watched.contains(&resolved_git_dir.join("logs/HEAD")));
        assert!(!watched.contains(&resolved_common_dir.join("refs/heads")));

        // Once the current branch is loose, watch its exact ref and stop
        // watching shared packed refs or the broader parent directory.
        let loose_ref = resolved_common_dir.join(&reference);
        std::fs::write(&loose_ref, "fedcba9876543210fedcba9876543210fedcba98\n").unwrap();
        let watched = git_watch_paths(&worktree.join(".git"));
        assert!(watched.contains(&loose_ref));
        assert!(!watched.contains(&resolved_common_dir.join("packed-refs")));
        assert!(!watched.contains(&resolved_common_dir.join("refs/heads")));

        // A detached checkout needs only its private HEAD file.
        std::fs::write(
            git_dir.join("HEAD"),
            "0123456789012345678901234567890123456789\n",
        )
        .unwrap();
        let watched = git_watch_paths(&worktree.join(".git"));
        assert_eq!(watched.len(), 1);
        assert_eq!(
            watched[0].canonicalize().unwrap(),
            git_dir.join("HEAD").canonicalize().unwrap()
        );
    }
}

fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn chrono_now_utc() -> String {
    // Use `date` command to avoid adding a build dependency
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| {
            // "rustc 1.85.0 (..." -> "1.85.0"
            s.trim()
                .strip_prefix("rustc ")
                .unwrap_or(s.trim())
                .split_whitespace()
                .next()
                .unwrap_or("unknown")
                .to_string()
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn git_dirty() -> String {
    Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| {
            if o.status.success() {
                let stdout = String::from_utf8_lossy(&o.stdout);
                if stdout.trim().is_empty() {
                    "false"
                } else {
                    "true"
                }
            } else {
                "unknown"
            }
        })
        .unwrap_or("unknown")
        .to_string()
}

fn spectral_rev() -> String {
    // Parse Cargo.lock for spectral's git rev
    let lock_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    if let Ok(contents) = std::fs::read_to_string(lock_path) {
        // Look for: name = "spectral"\nversion = ...\nsource = "git+...?rev=XXXX#full_hash"
        let mut in_spectral = false;
        for line in contents.lines() {
            if line.trim() == "name = \"spectral\"" {
                in_spectral = true;
                continue;
            }
            if in_spectral && line.starts_with("source = ") {
                // Extract short rev from the ?rev=XXX part
                if let Some(rev_start) = line.find("?rev=") {
                    // Safety: "?rev=" is pure ASCII so rev_start + 5 is always a valid UTF-8 boundary
                    #[allow(clippy::string_slice)]
                    let after = &line[rev_start + 5..];
                    let rev = after.split('#').next().unwrap_or(after);
                    let rev = rev.trim_end_matches('"');
                    return rev.to_string();
                }
                break;
            }
            if in_spectral && line.starts_with("[[") {
                break; // next package block
            }
        }
    }
    "unknown".to_string()
}
