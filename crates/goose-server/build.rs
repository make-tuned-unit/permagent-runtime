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
    let dot_git = std::path::Path::new("../../.git");
    let git_dir = if dot_git.is_file() {
        // Linked worktree: follow the pointer. This gitdir has its own HEAD,
        // which is what a checkout in this worktree rewrites.
        match std::fs::read_to_string(dot_git) {
            Ok(text) => text
                .lines()
                .find_map(|l| l.strip_prefix("gitdir:"))
                .map(|p| std::path::PathBuf::from(p.trim())),
            Err(_) => None,
        }
    } else if dot_git.is_dir() {
        Some(dot_git.to_path_buf())
    } else {
        None
    };

    let Some(git_dir) = git_dir else {
        // No git metadata (a source tarball, a vendored build). Nothing to
        // watch; the SHA falls back to "unknown" as it always has.
        return;
    };

    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());

    // In a worktree the refs live in the shared common dir, named by
    // `commondir` (usually `../..` relative to the gitdir).
    let common = std::fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|c| git_dir.join(c.trim()))
        .unwrap_or_else(|| git_dir.clone());
    println!("cargo:rerun-if-changed={}", common.join("refs").display());
    // A packed ref moves without any file under refs/ changing.
    println!(
        "cargo:rerun-if-changed={}",
        common.join("packed-refs").display()
    );
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
