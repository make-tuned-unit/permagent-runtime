//! Per-category filesystem scanners.
//!
//! Each scanner is a pure function that walks a set of paths and returns
//! structured findings. No shell commands, no agent involvement.

use super::safety;
use super::size;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// A single storage finding — matches the Finding struct contract in findings.rs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanFinding {
    pub id: String,
    #[serde(rename = "type")]
    pub finding_type: String,
    pub path: String,
    pub size_bytes: u64,
    pub age_days: Option<u64>,
    pub recommendation: String,
}

/// Summary stats for a scan category.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CategoryStats {
    pub count: u64,
    pub total_bytes: u64,
}

/// Complete scan result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanResult {
    pub run_id: String,
    pub findings: Vec<ScanFinding>,
    pub categories: std::collections::HashMap<String, CategoryStats>,
    pub total_bytes: u64,
}

/// Production home resolution. The scan bodies never read this (or any other
/// process-global state) directly: each public scanner is a thin delegator to
/// a `*_in(home, …)` variant that takes the scan root as a parameter, and
/// tests call the `_in` variants with a TempDir. Keeping `$HOME` out of the
/// scan path is what makes the tests race-free by construction — mutating the
/// global env under `#[serial]` (the pre-#462-fix approach) still flaked on
/// CI because `serial_test` only serializes against its own group, not against
/// env-touching tests in the config/provider `env_lock` domain.
fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
}

/// Resolve dev scan roots under `home`: probe conventional directories first,
/// fall back to `home` itself.
fn dev_scan_roots(home: &Path) -> Vec<(PathBuf, u32)> {
    let conventional = ["dev", "code", "src", "projects", "repos", "workspace"];

    let mut roots: Vec<(PathBuf, u32)> = Vec::new();
    for name in &conventional {
        // Both ~/dev and ~/Documents/dev. Nesting the code directory under
        // Documents is common, and on 2026-08-13 it made this scan report a
        // clean machine while 96 GB of cargo `target/` sat in
        // ~/Documents/dev/<repo>/target — the user's whole complaint was that
        // the scan "found nothing", and it had simply never looked there.
        for base in [home.to_path_buf(), home.join("Documents")] {
            let p = base.join(name);
            if p.is_dir() {
                roots.push((p, 6));
            }
        }
    }
    // Xcode's default, and GitHub Desktop's.
    for extra in ["Developer", "Documents/GitHub"] {
        let p = home.join(extra);
        if p.is_dir() {
            roots.push((p, 6));
        }
    }

    if roots.is_empty() {
        // Fallback: walk `home`, skipping irrelevant subdirs.
        //
        // Depth 6, not 3. A repo one folder deep — `Documents/dev/<repo>/target`
        // — needs four levels; a nested package such as
        // `Documents/dev/<repo>/ui/desktop/src-tauri/target` needs six, and
        // at 3 the walk stopped one short of exactly the directories this
        // scanner exists to find.
        roots.push((home.to_path_buf(), 6));
    }
    roots
}

/// Returns true if `dir_name` should be skipped during dev root scanning
/// (when falling back to $HOME traversal).
fn should_skip_dev_dir(path: &Path, home: &Path) -> bool {
    // Only skip at depth 1 from home
    if let Some(parent) = path.parent() {
        if parent == home {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Skip hidden dirs (start with .) at home level
                if name.starts_with('.') {
                    return true;
                }
                return safety::DEV_SCAN_SKIP_DIRS
                    .iter()
                    .any(|skip| name.eq_ignore_ascii_case(skip));
            }
        }
    }
    false
}

// ── Individual scanners ─────────────────────────────────────────

/// Minimum size before a cache dir is worth reporting.
const DEV_CACHE_MIN_BYTES: u64 = 10_000_000;

/// Scan for cargo target/ directories (with Cargo.toml sibling)
/// and node_modules/ directories (with package.json sibling).
///
/// Also reports cargo target *farms* that are not named `target` next to a
/// Cargo.toml — per-lane `.shared-target/<lane>` trees and Cursor sandbox
/// `cursor-sandbox-cache/*/cargo-target` dirs. Those two layouts held >100 GB
/// on 2026-08-21 while this scan reported a clean disk.
pub fn scan_dev_caches(counter: &mut u32) -> Vec<ScanFinding> {
    // Confirmed roots are resolved HERE, at the impure entry point, and never
    // inside `scan_dev_caches_in` — that function takes its root as a
    // parameter precisely so tests are hermetic (see the note above its tests
    // about injection removing shared global state). Reading Config::global()
    // or the real $HOME from inside it made three tests depend on the machine
    // they ran on.
    let confirmed = crate::config::dev_roots::dev_roots();
    let mut findings = if !confirmed.is_empty() {
        scan_dev_caches_in_roots(&confirmed, counter)
    } else {
        scan_dev_caches_in(&home_dir(), counter)
    };
    // Sidecar farms live next to worktrees and under $TMPDIR, not inside a
    // confirmed repo root. Always look — a confirmed-root-only walk is what
    // missed them in the first place.
    findings.extend(scan_sidecar_cargo_targets_in(
        &home_dir(),
        &std::env::temp_dir(),
        counter,
    ));
    dedup_findings_by_path(&mut findings);
    findings.sort_by_key(|f| std::cmp::Reverse(f.size_bytes));
    findings
}

/// Scan an explicit set of roots — the user's confirmed code directories.
fn scan_dev_caches_in_roots(roots: &[PathBuf], counter: &mut u32) -> Vec<ScanFinding> {
    let mut findings = Vec::new();
    for root in roots {
        findings.extend(scan_dev_caches_in(root, counter));
    }
    findings
}

fn scan_dev_caches_in(home: &Path, counter: &mut u32) -> Vec<ScanFinding> {
    let mut findings = Vec::new();
    let roots = dev_scan_roots(home);

    for (root, max_depth) in &roots {
        let walker = ignore::WalkBuilder::new(root)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .hidden(false)
            .max_depth(Some(*max_depth as usize))
            .build();

        for entry in walker.flatten() {
            let path = entry.path();

            // When scanning the home root itself, skip irrelevant top-level dirs
            if root.as_path() == home && should_skip_dev_dir(path, home) {
                continue;
            }

            if !entry.file_type().is_some_and(|ft| ft.is_dir()) {
                continue;
            }

            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };

            if safety::is_excluded(path) {
                continue;
            }

            // Per-lane cargo target farm. Each child is a CARGO_TARGET_DIR, not
            // a directory named `target` next to Cargo.toml — the old check
            // walked right past these.
            if name == ".shared-target" {
                findings.extend(scan_shared_target_lanes(path, counter));
                continue;
            }

            // Cargo target/ with Cargo.toml sibling
            if name == "target" {
                if let Some(parent) = path.parent() {
                    if parent.join("Cargo.toml").exists() {
                        push_dev_cache(&mut findings, counter, path, true);
                    }
                }
            }

            // node_modules/ with package.json sibling
            if name == "node_modules" {
                if let Some(parent) = path.parent() {
                    if parent.join("package.json").exists() {
                        push_dev_cache(&mut findings, counter, path, true);
                    }
                }
            }
        }
    }

    // Sort largest first
    findings.sort_by_key(|f| std::cmp::Reverse(f.size_bytes));
    findings
}

fn push_dev_cache(
    findings: &mut Vec<ScanFinding>,
    counter: &mut u32,
    path: &Path,
    check_exclude: bool,
) {
    if check_exclude && safety::is_excluded(path) {
        return;
    }
    let sz = size::dir_size(path);
    if sz <= DEV_CACHE_MIN_BYTES {
        return;
    }
    *counter += 1;
    findings.push(ScanFinding {
        id: format!("finding-{:03}", counter),
        finding_type: "dev_cache".to_string(),
        path: path.to_string_lossy().to_string(),
        size_bytes: sz,
        age_days: size::age_days(path),
        recommendation: safety::recommendation_for("dev_cache").to_string(),
    });
    debug!("dev_cache: {} ({} bytes)", path.display(), sz);
}

fn dedup_findings_by_path(findings: &mut Vec<ScanFinding>) {
    let mut seen = std::collections::HashSet::new();
    findings.retain(|f| seen.insert(f.path.clone()));
}

/// Report each lane under a `.shared-target` directory.
fn scan_shared_target_lanes(dir: &Path, counter: &mut u32) -> Vec<ScanFinding> {
    let mut findings = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return findings,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            push_dev_cache(&mut findings, counter, &path, true);
        }
    }
    findings
}

/// Cargo target farms that a repo-root walk never reaches: `.shared-target`
/// next to worktrees, and Cursor's `$TMPDIR/cursor-sandbox-cache/*/cargo-target`.
fn scan_sidecar_cargo_targets_in(home: &Path, tmp: &Path, counter: &mut u32) -> Vec<ScanFinding> {
    let mut findings = Vec::new();
    let conventional = ["dev", "code", "src", "projects", "repos", "workspace"];
    for name in &conventional {
        for base in [home.to_path_buf(), home.join("Documents")] {
            collect_shared_target_farms(&base.join(name), &mut findings, counter);
        }
    }
    findings.extend(scan_cursor_sandbox_targets_in(tmp, counter));
    findings
}

fn collect_shared_target_farms(
    code_root: &Path,
    findings: &mut Vec<ScanFinding>,
    counter: &mut u32,
) {
    if !code_root.is_dir() {
        return;
    }
    let direct = code_root.join(".shared-target");
    if direct.is_dir() {
        findings.extend(scan_shared_target_lanes(&direct, counter));
    }
    let entries = match fs::read_dir(code_root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let shared = entry.path().join(".shared-target");
        if shared.is_dir() {
            findings.extend(scan_shared_target_lanes(&shared, counter));
        }
    }
}

/// Cursor sandbox `CARGO_TARGET_DIR`s. `$TMPDIR/cursor-sandbox-cache` is
/// outside every code root, and on macOS it lives under `/var/folders`.
/// Skip `is_excluded` — `/private/var` is otherwise blocked, and these
/// directories are rebuildable caches, not secrets.
fn scan_cursor_sandbox_targets_in(tmp: &Path, counter: &mut u32) -> Vec<ScanFinding> {
    let mut findings = Vec::new();
    let cache = tmp.join("cursor-sandbox-cache");
    if !cache.is_dir() {
        return findings;
    }
    let entries = match fs::read_dir(&cache) {
        Ok(e) => e,
        Err(_) => return findings,
    };
    for entry in entries.flatten() {
        let cargo_target = entry.path().join("cargo-target");
        if cargo_target.is_dir() {
            push_dev_cache(&mut findings, counter, &cargo_target, false);
        }
    }
    findings
}

/// Scan app cache directories: ~/Library/Caches, ~/.cache, ~/.npm, ~/.cargo/registry.
pub fn scan_app_caches(counter: &mut u32) -> Vec<ScanFinding> {
    scan_app_caches_in(&home_dir(), counter)
}

fn scan_app_caches_in(home: &Path, counter: &mut u32) -> Vec<ScanFinding> {
    let cache_dirs = [
        home.join("Library/Caches"),
        home.join(".cache"),
        home.join(".npm"),
        home.join(".cargo/registry"),
    ];

    let mut findings = Vec::new();

    for cache_root in &cache_dirs {
        if !cache_root.is_dir() {
            continue;
        }

        // For Library/Caches and .cache, report top-level subdirectories
        // For .npm and .cargo/registry, report the directory itself
        if cache_root.ends_with("Library/Caches") || cache_root.ends_with(".cache") {
            let entries = match fs::read_dir(cache_root) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                if safety::is_excluded(&path) {
                    continue;
                }
                let sz = size::dir_size(&path);
                if sz > 50_000_000 {
                    // >50MB to be worth reporting
                    *counter += 1;
                    findings.push(ScanFinding {
                        id: format!("finding-{:03}", counter),
                        finding_type: "app_cache".to_string(),
                        path: path.to_string_lossy().to_string(),
                        size_bytes: sz,
                        age_days: size::age_days(&path),
                        recommendation: safety::recommendation_for("app_cache").to_string(),
                    });
                }
            }
        } else {
            // .npm, .cargo/registry — report as single entry
            let sz = size::dir_size(cache_root);
            if sz > 50_000_000 {
                *counter += 1;
                findings.push(ScanFinding {
                    id: format!("finding-{:03}", counter),
                    finding_type: "app_cache".to_string(),
                    path: cache_root.to_string_lossy().to_string(),
                    size_bytes: sz,
                    age_days: size::age_days(cache_root),
                    recommendation: safety::recommendation_for("app_cache").to_string(),
                });
            }
        }
    }

    findings.sort_by_key(|f| std::cmp::Reverse(f.size_bytes));
    findings
}

/// Scan Xcode DerivedData.
pub fn scan_xcode_derived(counter: &mut u32) -> Vec<ScanFinding> {
    scan_xcode_derived_in(&home_dir(), counter)
}

fn scan_xcode_derived_in(home: &Path, counter: &mut u32) -> Vec<ScanFinding> {
    let derived_data = home.join("Library/Developer/Xcode/DerivedData");
    let mut findings = Vec::new();

    if !derived_data.is_dir() {
        return findings;
    }

    let entries = match fs::read_dir(&derived_data) {
        Ok(e) => e,
        Err(_) => return findings,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let sz = size::dir_size(&path);
        if sz > 50_000_000 {
            *counter += 1;
            findings.push(ScanFinding {
                id: format!("finding-{:03}", counter),
                finding_type: "build_artifact".to_string(),
                path: path.to_string_lossy().to_string(),
                size_bytes: sz,
                age_days: size::age_days(&path),
                recommendation: safety::recommendation_for("build_artifact").to_string(),
            });
        }
    }

    findings.sort_by_key(|f| std::cmp::Reverse(f.size_bytes));
    findings
}

/// Scan ~/Downloads for files older than 30 days and >10MB.
pub fn scan_stale_downloads(counter: &mut u32) -> Vec<ScanFinding> {
    scan_stale_downloads_in(&home_dir(), counter)
}

fn scan_stale_downloads_in(home: &Path, counter: &mut u32) -> Vec<ScanFinding> {
    let downloads = home.join("Downloads");
    let mut findings = Vec::new();

    if !downloads.is_dir() {
        return findings;
    }

    let walker = ignore::WalkBuilder::new(&downloads)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .hidden(false)
        .max_depth(Some(3))
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path();
        if safety::is_excluded(path) || safety::is_icloud_evicted(path) {
            continue;
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let age = match meta.modified().ok().and_then(|m| m.elapsed().ok()) {
            Some(d) => d.as_secs() / 86400,
            None => continue,
        };

        if age >= 30 && meta.len() > 10_000_000 {
            *counter += 1;
            findings.push(ScanFinding {
                id: format!("finding-{:03}", counter),
                finding_type: "stale_file".to_string(),
                path: path.to_string_lossy().to_string(),
                size_bytes: meta.len(),
                age_days: Some(age),
                recommendation: safety::recommendation_for("stale_file").to_string(),
            });
        }
    }

    findings.sort_by_key(|f| std::cmp::Reverse(f.size_bytes));
    findings
}

/// Scan ~/Downloads, ~/Desktop, ~/Documents for files >100MB.
pub fn scan_large_user_files(counter: &mut u32) -> Vec<ScanFinding> {
    scan_large_user_files_in(&home_dir(), counter)
}

fn scan_large_user_files_in(home: &Path, counter: &mut u32) -> Vec<ScanFinding> {
    let dirs = [
        home.join("Downloads"),
        home.join("Desktop"),
        home.join("Documents"),
    ];
    let mut findings = Vec::new();

    for dir in &dirs {
        if !dir.is_dir() {
            continue;
        }

        let walker = ignore::WalkBuilder::new(dir)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .hidden(false)
            .max_depth(Some(3))
            .build();

        for entry in walker.flatten() {
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let path = entry.path();
            if safety::is_excluded(path) || safety::is_icloud_evicted(path) {
                continue;
            }

            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            if meta.len() > 100_000_000 {
                *counter += 1;
                findings.push(ScanFinding {
                    id: format!("finding-{:03}", counter),
                    finding_type: "large_file".to_string(),
                    path: path.to_string_lossy().to_string(),
                    size_bytes: meta.len(),
                    age_days: size::age_days(path),
                    recommendation: safety::recommendation_for("large_file").to_string(),
                });
            }
        }
    }

    findings.sort_by_key(|f| std::cmp::Reverse(f.size_bytes));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // These tests inject their TempDir as the scan root via the `*_in`
    // variants — no `$HOME` mutation, no serialization requirement. The old
    // versions set the process-global `HOME` under `#[serial]` (issue #462),
    // but that still flaked on CI: `serial_test` only serializes tests inside
    // its own group, and env-touching tests in the OTHER serialization domain
    // (the config/provider `env_lock` mutex) could clobber `HOME` mid-scan.
    // Injecting the root removes the shared global state instead of trying to
    // lock it — the race is gone by construction.

    /// A CONFIRMED root (what onboarding stores) is scanned directly, and does
    /// not have to look like a home directory. Without this the behaviour held
    /// only by accident: `scan_dev_caches_in` treats its argument as a home,
    /// finds no conventional child inside it, and survives via the fallback.
    #[test]
    fn a_confirmed_root_is_scanned_directly() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("permagent-runtime");
        fs::create_dir_all(project.join("target/debug")).unwrap();
        fs::write(project.join("Cargo.toml"), "[package]").unwrap();
        fs::write(project.join("target/debug/big.o"), vec![0u8; 11_000_000]).unwrap();

        let mut counter = 0;
        let findings = scan_dev_caches_in_roots(&[root.path().to_path_buf()], &mut counter);
        assert!(
            findings
                .iter()
                .any(|f| f.path.contains("permagent-runtime")),
            "a repo directly inside a confirmed root must be found: {findings:?}"
        );
    }

    /// Regression, 2026-08-13. Repos lived in ~/Documents/dev, which matched no
    /// conventional root, and the $HOME fallback only walked 3 deep — one level
    /// short of `Documents/dev/<repo>/target`. The scan reported a clean machine
    /// while 96 GB of build artifacts sat there.
    #[test]
    fn finds_cargo_target_when_repos_live_under_documents() {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join("Documents/dev/permagent-runtime");
        fs::create_dir_all(project.join("target/debug")).unwrap();
        fs::write(project.join("Cargo.toml"), "[package]").unwrap();
        // Above the dev-cache reporting floor (see the sibling test).
        let data = vec![0u8; 11_000_000];
        fs::write(project.join("target/debug/big.o"), &data).unwrap();

        let mut counter = 0;
        let findings = scan_dev_caches_in(home.path(), &mut counter);
        assert!(
            findings
                .iter()
                .any(|f| f.path.contains("permagent-runtime")),
            "a repo under Documents/dev must be found: {findings:?}"
        );
    }

    /// The same layout must also be reachable through the bare-$HOME fallback,
    /// which is what runs when no conventional root exists at all.
    #[test]
    fn home_fallback_reaches_one_folder_deeper_than_a_bare_repo() {
        let home = tempfile::tempdir().unwrap();
        // No conventional root anywhere, so dev_scan_roots falls back to $HOME.
        let project = home.path().join("Code Projects/client-work/api");
        fs::create_dir_all(project.join("target/debug")).unwrap();
        fs::write(project.join("Cargo.toml"), "[package]").unwrap();
        fs::write(project.join("target/debug/big.o"), vec![0u8; 11_000_000]).unwrap();

        let roots = dev_scan_roots(home.path());
        assert_eq!(roots.len(), 1, "expected the $HOME fallback: {roots:?}");
        assert!(
            roots[0].1 >= 6,
            "fallback must reach depth 6 (nested package targets), got {}",
            roots[0].1
        );
    }

    /// Regression, 2026-08-21. Per-lane `CARGO_TARGET_DIR` trees live at
    /// `<worktrees>/.shared-target/<lane>` — not named `target`, no sibling
    /// Cargo.toml. The scan that only looked for `target/` next to Cargo.toml
    /// reported a clean disk while tens of GB sat in those lanes.
    #[test]
    fn finds_shared_target_lanes_without_cargo_toml_sibling() {
        let home = TempDir::new().unwrap();
        let lane = home
            .path()
            .join("Documents/dev/permagent-worktrees/.shared-target/financier");
        fs::create_dir_all(&lane).unwrap();
        fs::write(lane.join("big.o"), vec![0u8; 11_000_000]).unwrap();

        let mut counter = 0;
        let findings = scan_dev_caches_in(home.path(), &mut counter);
        assert!(
            findings.iter().any(|f| f.path.contains("financier")),
            "a .shared-target lane must be found without Cargo.toml: {findings:?}"
        );

        // Sidecar discovery must also find it when the walk root is a *repo*
        // (confirmed roots never visit the sibling worktrees directory).
        let mut counter = 0;
        let empty_tmp = TempDir::new().unwrap();
        let sidecar = scan_sidecar_cargo_targets_in(home.path(), empty_tmp.path(), &mut counter);
        assert!(
            sidecar.iter().any(|f| f.path.contains("financier")),
            "sidecar scan must find .shared-target next to worktrees: {sidecar:?}"
        );
    }

    /// Regression, 2026-08-21. Cursor sandboxes set CARGO_TARGET_DIR to
    /// `$TMPDIR/cursor-sandbox-cache/<id>/cargo-target`, outside every code
    /// root. One of these held 105 GB while the weekly scan missed it.
    #[test]
    fn finds_cursor_sandbox_cargo_target() {
        let tmp = TempDir::new().unwrap();
        let cargo_target = tmp
            .path()
            .join("cursor-sandbox-cache/af74febe857c7047aab2841aad58aeaa/cargo-target");
        fs::create_dir_all(&cargo_target).unwrap();
        fs::write(cargo_target.join("big.o"), vec![0u8; 11_000_000]).unwrap();

        let mut counter = 0;
        let findings = scan_cursor_sandbox_targets_in(tmp.path(), &mut counter);
        assert_eq!(
            findings.len(),
            1,
            "expected the sandbox cargo-target: {findings:?}"
        );
        assert!(findings[0].path.contains("cargo-target"));
        assert!(findings[0].size_bytes >= 11_000_000);
    }

    #[test]
    fn test_dev_cache_finds_target_with_cargo_toml() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("myproject");
        fs::create_dir_all(project.join("target/debug")).unwrap();
        fs::write(project.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        // Write >10MB of data
        let data = vec![0u8; 11_000_000];
        fs::write(project.join("target/debug/big.o"), &data).unwrap();

        let mut counter = 0;
        let findings = scan_dev_caches_in(tmp.path(), &mut counter);
        assert!(!findings.is_empty(), "should find cargo target/");
        assert_eq!(findings[0].finding_type, "dev_cache");
        assert!(findings[0].path.contains("target"));
        assert!(findings[0].size_bytes >= 11_000_000);
    }

    #[test]
    fn test_dev_cache_scans_conventional_root() {
        // A conventional dev dir (~/dev) present under the injected root must
        // be preferred over the home-fallback walk (the non-fallback branch of
        // `dev_scan_roots`).
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("dev/myproject");
        fs::create_dir_all(project.join("target")).unwrap();
        fs::write(project.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        let data = vec![0u8; 11_000_000];
        fs::write(project.join("target/big.o"), &data).unwrap();

        let mut counter = 0;
        let findings = scan_dev_caches_in(tmp.path(), &mut counter);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].path.contains("dev"));
        assert!(findings[0].path.contains("target"));
    }

    #[test]
    fn test_dev_cache_ignores_target_without_cargo_toml() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("notaproject");
        fs::create_dir_all(project.join("target")).unwrap();
        let data = vec![0u8; 11_000_000];
        fs::write(project.join("target/big.o"), &data).unwrap();
        // No Cargo.toml

        let mut counter = 0;
        let findings = scan_dev_caches_in(tmp.path(), &mut counter);
        assert!(
            findings.is_empty(),
            "should not find target/ without Cargo.toml"
        );
    }

    #[test]
    fn test_stale_downloads_respects_mtime() {
        let tmp = TempDir::new().unwrap();
        let downloads = tmp.path().join("Downloads");
        fs::create_dir_all(&downloads).unwrap();

        // Create a fresh file >10MB — should NOT be found (age < 30 days)
        let data = vec![0u8; 11_000_000];
        fs::write(downloads.join("fresh.zip"), &data).unwrap();

        let mut counter = 0;
        let findings = scan_stale_downloads_in(tmp.path(), &mut counter);
        assert!(findings.is_empty(), "fresh file should not be flagged");
    }

    #[test]
    fn test_large_user_files() {
        let tmp = TempDir::new().unwrap();
        let desktop = tmp.path().join("Desktop");
        fs::create_dir_all(&desktop).unwrap();

        // Create file >100MB
        let data = vec![0u8; 101_000_000];
        fs::write(desktop.join("huge.iso"), &data).unwrap();

        // Create file <100MB — should not be found
        let small = vec![0u8; 50_000_000];
        fs::write(desktop.join("medium.zip"), &small).unwrap();

        let mut counter = 0;
        let findings = scan_large_user_files_in(tmp.path(), &mut counter);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].path.contains("huge.iso"));
        assert!(findings[0].size_bytes >= 101_000_000);
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let mut counter = 0;
        assert!(scan_dev_caches_in(tmp.path(), &mut counter).is_empty());
        assert!(scan_app_caches_in(tmp.path(), &mut counter).is_empty());
        assert!(scan_xcode_derived_in(tmp.path(), &mut counter).is_empty());
        assert!(scan_stale_downloads_in(tmp.path(), &mut counter).is_empty());
        assert!(scan_large_user_files_in(tmp.path(), &mut counter).is_empty());
    }
}
