//! Per-category filesystem scanners.
//!
//! Each scanner is a pure function that walks a set of paths and returns
//! structured findings. No shell commands, no agent involvement.

use super::classify::{self, Classification};
use super::inuse::{InUseConfig, LsofProbe, NoProcessProbe, ProcessProbe};
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
    /// Human-facing verdict, derived from `category` — see `classify.rs`.
    pub recommendation: String,
    /// Stable category key: one of `classify::CAT_*`. This, not the finding
    /// *type*, is what the action path enforces on.
    #[serde(default = "default_category")]
    pub category: String,
    /// One line naming what removing this costs: "5 rustc processes are
    /// compiling here", "re-downloads 1.2 GB the next time cargo builds".
    #[serde(default)]
    pub consequence: Option<String>,
}

/// Ledgers written before categories existed have no `category` field. They
/// predate the classifier, so nothing about them was verified — treat them as
/// needing review rather than inheriting a "safe" they were never given.
fn default_category() -> String {
    classify::CAT_REVIEW.to_string()
}

/// Everything a scan needs beyond the filesystem: the in-use window and the
/// process probe.
///
/// Constructed at the impure entry point (`storage_health::run_scan`) and
/// passed down, so no scan body reads process-global state and every test can
/// pin the machine's apparent liveness — see the note above `home_dir`.
pub struct ScanContext {
    pub in_use: InUseConfig,
    pub probe: Box<dyn ProcessProbe>,
}

impl ScanContext {
    /// The real machine: `lsof` and the user's configured mtime window.
    pub fn production() -> Self {
        Self {
            in_use: InUseConfig::from_config(),
            probe: Box::new(LsofProbe),
        }
    }

    /// A machine with nothing running. Fixture files are written milliseconds
    /// before they are scanned, so the mtime probe is disabled too — otherwise
    /// every scanner test would report its own fixtures as live builds.
    pub fn idle() -> Self {
        Self {
            in_use: InUseConfig {
                recent_mtime_minutes: 0,
                ..InUseConfig::default()
            },
            probe: Box::new(NoProcessProbe),
        }
    }

    fn classify(&self, finding_type: &str, path: &Path, size_bytes: u64) -> Classification {
        classify::classify(
            finding_type,
            path,
            size_bytes,
            &self.in_use,
            self.probe.as_ref(),
        )
    }
}

impl Default for ScanContext {
    fn default() -> Self {
        Self::idle()
    }
}

/// Build one finding, with its category and consequence already decided.
fn make_finding(
    ctx: &ScanContext,
    counter: &mut u32,
    finding_type: &str,
    path: &Path,
    size_bytes: u64,
    age_days: Option<u64>,
) -> ScanFinding {
    // WHY: macOS TMPDIR commonly uses /var -> /private/var. Emit the actual
    // path so action-time rejection of symlink aliases does not invalidate
    // the scanner's explicitly supported Cursor sandbox targets.
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path = canonical.as_path();
    let c = ctx.classify(finding_type, path, size_bytes);
    *counter += 1;
    ScanFinding {
        id: format!("finding-{:03}", counter),
        finding_type: finding_type.to_string(),
        path: path.to_string_lossy().to_string(),
        size_bytes,
        age_days,
        recommendation: c.recommendation,
        category: c.category.to_string(),
        consequence: c.consequence,
    }
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
pub fn scan_dev_caches(ctx: &ScanContext, counter: &mut u32) -> Vec<ScanFinding> {
    // Confirmed roots are resolved HERE, at the impure entry point, and never
    // inside `scan_dev_caches_in` — that function takes its root as a
    // parameter precisely so tests are hermetic (see the note above its tests
    // about injection removing shared global state). Reading Config::global()
    // or the real $HOME from inside it made three tests depend on the machine
    // they ran on.
    let confirmed = crate::config::dev_roots::dev_roots();
    let mut findings = if !confirmed.is_empty() {
        scan_dev_caches_in_roots(&confirmed, ctx, counter)
    } else {
        scan_dev_caches_in(&home_dir(), ctx, counter)
    };
    // Sidecar farms live next to worktrees and under $TMPDIR, not inside a
    // confirmed repo root. Always look — a confirmed-root-only walk is what
    // missed them in the first place.
    findings.extend(scan_sidecar_cargo_targets_in(
        &home_dir(),
        &std::env::temp_dir(),
        ctx,
        counter,
    ));
    dedup_findings_by_path(&mut findings);
    findings.sort_by_key(|f| std::cmp::Reverse(f.size_bytes));
    findings
}

/// Scan an explicit set of roots — the user's confirmed code directories.
fn scan_dev_caches_in_roots(
    roots: &[PathBuf],
    ctx: &ScanContext,
    counter: &mut u32,
) -> Vec<ScanFinding> {
    let mut findings = Vec::new();
    for root in roots {
        findings.extend(scan_dev_caches_in(root, ctx, counter));
    }
    findings
}

fn scan_dev_caches_in(home: &Path, ctx: &ScanContext, counter: &mut u32) -> Vec<ScanFinding> {
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
                findings.extend(scan_shared_target_lanes(path, ctx, counter));
                continue;
            }

            // Cargo target/ with Cargo.toml sibling
            if name == "target" {
                if let Some(parent) = path.parent() {
                    if parent.join("Cargo.toml").exists() {
                        push_dev_cache(&mut findings, ctx, counter, path, true);
                    }
                }
            }

            // node_modules/ with package.json sibling
            if name == "node_modules" {
                if let Some(parent) = path.parent() {
                    if parent.join("package.json").exists() {
                        push_dev_cache(&mut findings, ctx, counter, path, true);
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
    ctx: &ScanContext,
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
    let f = make_finding(ctx, counter, "dev_cache", path, sz, size::age_days(path));
    debug!(
        "dev_cache: {} ({} bytes) [{}]",
        path.display(),
        sz,
        f.category
    );
    findings.push(f);
}

fn dedup_findings_by_path(findings: &mut Vec<ScanFinding>) {
    let mut seen = std::collections::HashSet::new();
    findings.retain(|f| seen.insert(f.path.clone()));
}

/// Report each lane under a `.shared-target` directory.
fn scan_shared_target_lanes(dir: &Path, ctx: &ScanContext, counter: &mut u32) -> Vec<ScanFinding> {
    let mut findings = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return findings,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            push_dev_cache(&mut findings, ctx, counter, &path, true);
        }
    }
    findings
}

/// Cargo target farms that a repo-root walk never reaches: `.shared-target`
/// next to worktrees, and Cursor's `$TMPDIR/cursor-sandbox-cache/*/cargo-target`.
fn scan_sidecar_cargo_targets_in(
    home: &Path,
    tmp: &Path,
    ctx: &ScanContext,
    counter: &mut u32,
) -> Vec<ScanFinding> {
    let mut findings = Vec::new();
    let conventional = ["dev", "code", "src", "projects", "repos", "workspace"];
    for name in &conventional {
        for base in [home.to_path_buf(), home.join("Documents")] {
            collect_shared_target_farms(&base.join(name), ctx, &mut findings, counter);
        }
    }
    findings.extend(scan_cursor_sandbox_targets_in(tmp, ctx, counter));
    findings
}

fn collect_shared_target_farms(
    code_root: &Path,
    ctx: &ScanContext,
    findings: &mut Vec<ScanFinding>,
    counter: &mut u32,
) {
    if !code_root.is_dir() {
        return;
    }
    let direct = code_root.join(".shared-target");
    if direct.is_dir() {
        findings.extend(scan_shared_target_lanes(&direct, ctx, counter));
    }
    let entries = match fs::read_dir(code_root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let shared = entry.path().join(".shared-target");
        if shared.is_dir() {
            findings.extend(scan_shared_target_lanes(&shared, ctx, counter));
        }
    }
}

/// Cursor sandbox `CARGO_TARGET_DIR`s. `$TMPDIR/cursor-sandbox-cache` is
/// outside every code root, and on macOS it lives under `/var/folders`.
/// Skip `is_excluded` — `/private/var` is otherwise blocked, and these
/// directories are rebuildable caches, not secrets.
fn scan_cursor_sandbox_targets_in(
    tmp: &Path,
    ctx: &ScanContext,
    counter: &mut u32,
) -> Vec<ScanFinding> {
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
            push_dev_cache(&mut findings, ctx, counter, &cargo_target, false);
        }
    }
    findings
}

/// Scan app cache directories: ~/Library/Caches, ~/.cache, ~/.npm, ~/.cargo/registry.
pub fn scan_app_caches(ctx: &ScanContext, counter: &mut u32) -> Vec<ScanFinding> {
    scan_app_caches_in(&home_dir(), ctx, counter)
}

fn scan_app_caches_in(home: &Path, ctx: &ScanContext, counter: &mut u32) -> Vec<ScanFinding> {
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
                    findings.push(make_finding(
                        ctx,
                        counter,
                        "app_cache",
                        &path,
                        sz,
                        size::age_days(&path),
                    ));
                }
            }
        } else {
            // .npm, .cargo/registry — report as single entry
            let sz = size::dir_size(cache_root);
            if sz > 50_000_000 {
                findings.push(make_finding(
                    ctx,
                    counter,
                    "app_cache",
                    cache_root,
                    sz,
                    size::age_days(cache_root),
                ));
            }
        }
    }

    findings.sort_by_key(|f| std::cmp::Reverse(f.size_bytes));
    findings
}

/// Scan Xcode DerivedData.
pub fn scan_xcode_derived(ctx: &ScanContext, counter: &mut u32) -> Vec<ScanFinding> {
    scan_xcode_derived_in(&home_dir(), ctx, counter)
}

fn scan_xcode_derived_in(home: &Path, ctx: &ScanContext, counter: &mut u32) -> Vec<ScanFinding> {
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
            findings.push(make_finding(
                ctx,
                counter,
                "build_artifact",
                &path,
                sz,
                size::age_days(&path),
            ));
        }
    }

    findings.sort_by_key(|f| std::cmp::Reverse(f.size_bytes));
    findings
}

/// Scan ~/Downloads for files older than 30 days and >10MB.
pub fn scan_stale_downloads(ctx: &ScanContext, counter: &mut u32) -> Vec<ScanFinding> {
    scan_stale_downloads_in(&home_dir(), ctx, counter)
}

fn scan_stale_downloads_in(home: &Path, ctx: &ScanContext, counter: &mut u32) -> Vec<ScanFinding> {
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
            findings.push(make_finding(
                ctx,
                counter,
                "stale_file",
                path,
                meta.len(),
                Some(age),
            ));
        }
    }

    findings.sort_by_key(|f| std::cmp::Reverse(f.size_bytes));
    findings
}

/// Scan ~/Downloads, ~/Desktop, ~/Documents for files >100MB.
pub fn scan_large_user_files(ctx: &ScanContext, counter: &mut u32) -> Vec<ScanFinding> {
    scan_large_user_files_in(&home_dir(), ctx, counter)
}

fn scan_large_user_files_in(home: &Path, ctx: &ScanContext, counter: &mut u32) -> Vec<ScanFinding> {
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
                findings.push(make_finding(
                    ctx,
                    counter,
                    "large_file",
                    path,
                    meta.len(),
                    size::age_days(path),
                ));
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

        let ctx = ScanContext::idle();
        let mut counter = 0;
        let findings = scan_dev_caches_in_roots(&[root.path().to_path_buf()], &ctx, &mut counter);
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

        let ctx = ScanContext::idle();
        let mut counter = 0;
        let findings = scan_dev_caches_in(home.path(), &ctx, &mut counter);
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

        let ctx = ScanContext::idle();
        let mut counter = 0;
        let findings = scan_dev_caches_in(home.path(), &ctx, &mut counter);
        assert!(
            findings.iter().any(|f| f.path.contains("financier")),
            "a .shared-target lane must be found without Cargo.toml: {findings:?}"
        );

        // Sidecar discovery must also find it when the walk root is a *repo*
        // (confirmed roots never visit the sibling worktrees directory).
        let ctx = ScanContext::idle();
        let mut counter = 0;
        let empty_tmp = TempDir::new().unwrap();
        let sidecar =
            scan_sidecar_cargo_targets_in(home.path(), empty_tmp.path(), &ctx, &mut counter);
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

        let ctx = ScanContext::idle();
        let mut counter = 0;
        let findings = scan_cursor_sandbox_targets_in(tmp.path(), &ctx, &mut counter);
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

        let ctx = ScanContext::idle();
        let mut counter = 0;
        let findings = scan_dev_caches_in(tmp.path(), &ctx, &mut counter);
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

        let ctx = ScanContext::idle();
        let mut counter = 0;
        let findings = scan_dev_caches_in(tmp.path(), &ctx, &mut counter);
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

        let ctx = ScanContext::idle();
        let mut counter = 0;
        let findings = scan_dev_caches_in(tmp.path(), &ctx, &mut counter);
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

        let ctx = ScanContext::idle();
        let mut counter = 0;
        let findings = scan_stale_downloads_in(tmp.path(), &ctx, &mut counter);
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

        let ctx = ScanContext::idle();
        let mut counter = 0;
        let findings = scan_large_user_files_in(tmp.path(), &ctx, &mut counter);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].path.contains("huge.iso"));
        assert!(findings[0].size_bytes >= 101_000_000);
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let ctx = ScanContext::idle();
        let mut counter = 0;
        assert!(scan_dev_caches_in(tmp.path(), &ctx, &mut counter).is_empty());
        assert!(scan_app_caches_in(tmp.path(), &ctx, &mut counter).is_empty());
        assert!(scan_xcode_derived_in(tmp.path(), &ctx, &mut counter).is_empty());
        assert!(scan_stale_downloads_in(tmp.path(), &ctx, &mut counter).is_empty());
        assert!(scan_large_user_files_in(tmp.path(), &ctx, &mut counter).is_empty());
    }
}

/// Sentinel around the scanner's own findings array. Present only when the
/// scan tool could not write the ledger itself. The scheduler reads this from
/// the tool result. Assistant prose is never parsed for totals.
pub const SCAN_FINDINGS_START: &str = "\n__PERMAGENT_SCAN_FINDINGS__\n";
pub const SCAN_FINDINGS_END: &str = "\n__END_PERMAGENT_SCAN_FINDINGS__\n";

/// Rows the scanner produced that could not become cleanup items.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedFinding {
    pub id: String,
    pub path: Option<String>,
    pub reason: String,
}

/// What a structured save kept. `saved` includes contained and blocked rows;
/// they stay visible and are not independently removable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructuredPersist {
    pub saved: usize,
    pub contained: usize,
    pub blocked: usize,
    pub rejected: Vec<RejectedFinding>,
}

pub fn embed_scan_findings_payload(findings_json: &str) -> String {
    format!("{SCAN_FINDINGS_START}{findings_json}{SCAN_FINDINGS_END}")
}

/// The scanner JSON between the sentinels. `None` when the tool did not embed
/// one (the ledger was already written, or this is ordinary prose).
pub fn extract_scan_findings_payload(text: &str) -> Option<Vec<serde_json::Value>> {
    let (_, rest) = text.split_once(SCAN_FINDINGS_START)?;
    let (json, _) = rest.split_once(SCAN_FINDINGS_END)?;
    serde_json::from_str(json.trim()).ok()
}

fn valid_run_id(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn ledger_path(dir: &Path, run_id: &str) -> PathBuf {
    dir.join(format!("{run_id}.json"))
}

/// A previous validated ledger for this run, if the file is the one the UI
/// will trust. Returns the finding count.
pub fn validated_ledger_count(dir: &Path, run_id: &str) -> Option<usize> {
    existing_validated_persist(dir, run_id).map(|saved| saved.saved)
}

/// Read a validated ledger without writing. Corrupt, foreign, and unverified
/// files are `None` so a later create refuses to replace them.
fn existing_validated_persist(dir: &Path, run_id: &str) -> Option<StructuredPersist> {
    if !valid_run_id(run_id) {
        return None;
    }
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(ledger_path(dir, run_id)).ok()?).ok()?;
    if value.get("run_id").and_then(|v| v.as_str()) != Some(run_id) {
        return None;
    }
    if value.get("validated").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let findings = value.get("findings").and_then(|v| v.as_array())?;
    let rejected = value
        .get("rejected")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    Some(StructuredPersist {
        saved: findings.len(),
        contained: findings
            .iter()
            .filter(|row| row.get("contained_in").is_some())
            .count(),
        blocked: findings
            .iter()
            .filter(|row| row.get("blocked_reason").is_some())
            .count(),
        rejected,
    })
}

/// Live name is unprefixed (`unprefixed_tools: true` on the storage_health
/// platform extension). `public_tools` emits `{extension}__{tool}` when that
/// flag is off. Any other prefix is a different tool and is not a scan result.
pub fn is_storage_scan_tool(name: &str) -> bool {
    const TOOL: &str = "scan_storage_health";
    if name == TOOL {
        return true;
    }
    match name.split_once("__") {
        Some((extension, tool)) => {
            extension == "storage_health" && tool == TOOL && !tool.contains("__")
        }
        None => false,
    }
}

fn stamp_row(
    row: &mut serde_json::Value,
    home: &Path,
    probe: &dyn super::inuse::action::ActivityProbe,
    observed_at: &str,
) -> Result<(), String> {
    use super::inuse::action::{current_classification, validate_path};
    let f: ScanFinding = serde_json::from_value(row.clone()).map_err(|e| e.to_string())?;
    if f.id.is_empty() {
        return Err("empty finding id".into());
    }
    if !row
        .get("category")
        .and_then(|v| v.as_str())
        .is_some_and(|c| classify::ALL_CATEGORIES.contains(&c))
    {
        return Err("unknown or missing category".into());
    }
    if !row
        .get("action_taken")
        .is_none_or(serde_json::Value::is_null)
    {
        return Err("pre-actioned finding".into());
    }
    let path = PathBuf::from(&f.path);
    validate_path(&f.finding_type, &path, home)?;
    let mut c = current_classification(&f.finding_type, &path, f.size_bytes, probe)?;
    // WHY: an agent must not erase a scanner's live-use observation while
    // narrating it. Only the explicit action-time recheck may release it.
    if f.category == classify::CAT_IN_USE {
        c.category = classify::CAT_IN_USE;
        c.recommendation = classify::category_label(classify::CAT_IN_USE).into();
        c.consequence = f.consequence.or(Some(
            "Scanner observed activity; recheck availability".into(),
        ));
    }
    row["category"] = serde_json::json!(c.category);
    row["recommendation"] = serde_json::json!(c.recommendation);
    row["consequence"] = serde_json::json!(c.consequence);
    row["observed_at"] = serde_json::json!(observed_at);
    Ok(())
}

fn row_path(row: &serde_json::Value) -> PathBuf {
    PathBuf::from(row.get("path").and_then(|v| v.as_str()).unwrap_or(""))
}

fn row_protected(row: &serde_json::Value) -> bool {
    matches!(
        row.get("category").and_then(|v| v.as_str()),
        Some(classify::CAT_IN_USE | classify::CAT_MACOS)
    )
}

/// Keep overlapping scanner rows. A nested or duplicate path is marked
/// `contained_in` so it is listed once as included in its parent. An ancestor
/// of an in-use or macOS-managed path is blocked instead of becoming a
/// shortcut that would delete the protected child.
fn relate_overlaps(rows: &mut [serde_json::Value]) {
    let info: Vec<(usize, PathBuf, bool)> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| (i, row_path(row), row_protected(row)))
        .collect();
    for (i, path, protected) in &info {
        if *protected {
            continue;
        }
        if info
            .iter()
            .any(|(j, other, is_protected)| *is_protected && i != j && other == path)
        {
            rows[*i]["blocked_reason"] =
                serde_json::json!("Same path as an in-use or protected item");
        } else if info.iter().any(|(_, other, is_protected)| {
            *is_protected && other != path && other.starts_with(path)
        }) {
            rows[*i]["blocked_reason"] = serde_json::json!("Contains an in-use or protected item");
        }
    }
    for (i, path, _) in &info {
        if rows[*i].get("blocked_reason").is_some() {
            continue;
        }
        let mut container: Option<(usize, usize)> = None;
        let mut exact = false;
        for (j, other, _) in &info {
            if i == j {
                continue;
            }
            if path == other && *i > *j {
                rows[*i]["contained_in"] = rows[*j]["id"].clone();
                exact = true;
                break;
            }
            if path != other && path.starts_with(other) {
                let depth = other.components().count();
                if container.is_none_or(|(_, found)| depth > found) {
                    container = Some((*j, depth));
                }
            }
        }
        if !exact {
            if let Some((j, _)) = container {
                rows[*i]["contained_in"] = rows[j]["id"].clone();
            }
        }
    }
}

enum CreateLedger {
    Created,
    /// The destination was already there. The temp file is dropped, so the
    /// existing bytes — actioned, validated, or corrupt — stay as they are.
    Present,
}

/// Create the ledger exactly once. There is no check-then-replace window:
/// a Trash/Keep recorded after the first create cannot be overwritten by a
/// later scan of the same run. A new scan uses a new session id.
fn create_ledger(
    dir: &Path,
    run_id: &str,
    ledger: &serde_json::Value,
) -> anyhow::Result<CreateLedger> {
    fs::create_dir_all(dir)?;
    let dest = ledger_path(dir, run_id);
    let mut pending = tempfile::NamedTempFile::new_in(dir)?;
    use std::io::Write;
    pending.write_all(&serde_json::to_vec_pretty(ledger)?)?;
    pending.as_file().sync_all()?;
    match pending.persist_noclobber(dest) {
        Ok(_) => Ok(CreateLedger::Created),
        Err(err) if err.error.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(CreateLedger::Present)
        }
        Err(err) => Err(anyhow::Error::from(err.error)),
    }
}

/// Empty input: nothing was found. Distinct from a scan whose rows all failed
/// validation.
const EMPTY_SCAN_MESSAGE: &str = "Scan complete — no cleanup candidates found";
/// Every scanner row was kept out of the selectable list. Not an empty scan,
/// and not a set of Trash targets.
const ALL_REJECTED_MESSAGE: &str = "No actionable cleanup findings — scanned items were rejected.";

/// Save the scanner's structured rows for this run, once.
///
/// Invalid rows are recorded in `rejected` and the rest are still saved.
/// When every row is rejected, the ledger is still written: `findings` is
/// empty and `rejected` keeps each reason. `validated` means that selectable
/// subset was accepted (it may be empty). Rejected paths are not findings and
/// are not safe to trash. A scan with no rows uses `EMPTY_SCAN_MESSAGE`
/// instead, so the two outcomes stay distinct.
/// A validated ledger for this run is reused and not rewritten, including
/// when it already records Trash/Keep. A corrupt or unverified file is left
/// untouched. Overlaps stay in the ledger and are marked so the action path
/// cannot delete a parent that contains a protected child, or delete the
/// same bytes twice.
pub fn persist_structured_findings(
    findings: &[ScanFinding],
    run_id: &str,
    dir: &Path,
    home: &Path,
    probe: &dyn super::inuse::action::ActivityProbe,
) -> anyhow::Result<StructuredPersist> {
    let rows = findings
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    persist_structured_values(&rows, run_id, dir, home, probe)
}

pub fn persist_structured_values(
    findings: &[serde_json::Value],
    run_id: &str,
    dir: &Path,
    home: &Path,
    probe: &dyn super::inuse::action::ActivityProbe,
) -> anyhow::Result<StructuredPersist> {
    anyhow::ensure!(valid_run_id(run_id), "invalid run id");
    if let Some(existing) = existing_validated_persist(dir, run_id) {
        return Ok(existing);
    }
    let observed_at = chrono::Utc::now().to_rfc3339();
    let mut kept: Vec<serde_json::Value> = Vec::new();
    let mut rejected = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for row in findings {
        let id = row
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let path = row.get("path").and_then(|v| v.as_str()).map(str::to_owned);
        if !id.is_empty() && !ids.insert(id.clone()) {
            rejected.push(RejectedFinding {
                id,
                path,
                reason: "duplicate finding id".into(),
            });
            continue;
        }
        let mut stamped = row.clone();
        match stamp_row(&mut stamped, home, probe, &observed_at) {
            Ok(()) => kept.push(stamped),
            Err(reason) => rejected.push(RejectedFinding { id, path, reason }),
        }
    }
    relate_overlaps(&mut kept);
    let contained = kept
        .iter()
        .filter(|row| row.get("contained_in").is_some())
        .count();
    let blocked = kept
        .iter()
        .filter(|row| row.get("blocked_reason").is_some())
        .count();
    let saved = kept.len();
    let ledger = serde_json::json!({
        "run_id": run_id,
        "created_at": observed_at,
        "findings": kept,
        "rejected": rejected.iter().map(|row| serde_json::json!({
            "id": row.id, "path": row.path, "reason": row.reason,
        })).collect::<Vec<_>>(),
        "validated": true,
        "message": if saved == 0 && !rejected.is_empty() {
            ALL_REJECTED_MESSAGE
        } else if saved == 0 {
            EMPTY_SCAN_MESSAGE
        } else {
            "Scan complete"
        },
    });
    let created = StructuredPersist {
        saved,
        contained,
        blocked,
        rejected,
    };
    match create_ledger(dir, run_id, &ledger)? {
        CreateLedger::Created => Ok(created),
        CreateLedger::Present => existing_validated_persist(dir, run_id).ok_or_else(|| {
            anyhow::anyhow!("ledger already exists and is not a validated scan for this run")
        }),
    }
}

/// Phase 5 save for a `produces_findings` recipe.
///
/// A validated ledger the scanner already wrote wins. Otherwise the scanner
/// payload embedded in the tool result is saved. Assistant `<findings>` tags
/// are the last resort. Prose totals are never turned into findings.
pub fn save_declared_storage_report(
    assistant_output: &str,
    tool_texts: &[String],
    run_id: &str,
    dir: &Path,
    home: &Path,
    probe: &dyn super::inuse::action::ActivityProbe,
) -> anyhow::Result<usize> {
    if let Some(count) = validated_ledger_count(dir, run_id) {
        return Ok(count);
    }
    if let Some(rows) = tool_texts
        .iter()
        .rev()
        .find_map(|text| extract_scan_findings_payload(text))
    {
        return persist_structured_values(&rows, run_id, dir, home, probe).map(|saved| saved.saved);
    }
    persist_report(assistant_output, run_id, dir, home, probe)
}

/// Validate a complete emitted ledger before making a new report visible.
/// The previous run is never overwritten, including on extraction/write failure.
pub fn persist_report(
    output: &str,
    run_id: &str,
    dir: &Path,
    home: &Path,
    probe: &dyn super::inuse::action::ActivityProbe,
) -> anyhow::Result<usize> {
    anyhow::ensure!(valid_run_id(run_id), "invalid run id");
    let (_, rest) = output
        .split_once("<findings>")
        .ok_or_else(|| anyhow::anyhow!("missing findings"))?;
    let (json, tail) = rest
        .split_once("</findings>")
        .ok_or_else(|| anyhow::anyhow!("unclosed findings"))?;
    anyhow::ensure!(
        !tail.contains("<findings>") && !json.contains("<findings>"),
        "ambiguous findings"
    );
    let mut rows: Vec<serde_json::Value> = serde_json::from_str(json)?;
    let mut ids = std::collections::HashSet::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    let observed_at = chrono::Utc::now().to_rfc3339();
    for row in &mut rows {
        let f: ScanFinding = serde_json::from_value(row.clone())?;
        anyhow::ensure!(
            !f.id.is_empty() && ids.insert(f.id.clone()),
            "duplicate or empty finding id"
        );
        stamp_row(row, home, probe, &observed_at).map_err(anyhow::Error::msg)?;
        let path = PathBuf::from(&f.path);
        anyhow::ensure!(
            !paths
                .iter()
                .any(|p| path.starts_with(p) || p.starts_with(&path)),
            "duplicate or overlapping paths"
        );
        paths.push(path);
    }
    let count = rows.len();
    let ledger = serde_json::json!({
        "run_id": run_id, "created_at": observed_at, "findings": rows, "validated": true,
        "message": if count == 0 { EMPTY_SCAN_MESSAGE } else { "Scan complete" },
    });
    match create_ledger(dir, run_id, &ledger)? {
        CreateLedger::Created => Ok(count),
        CreateLedger::Present => anyhow::bail!("ledger already exists"),
    }
}

#[cfg(test)]
mod st1_report_tests {
    use super::*;
    use crate::storage_health::inuse::{action::ActivityProbe, ProcessRef};
    struct Idle;
    impl ActivityProbe for Idle {
        fn check(&self, _: &Path) -> Result<Vec<ProcessRef>, String> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn acceptance_8_missing_malformed_and_unpersistable_reports_fail_without_replacing_previous() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let dir = home.join("findings");
        assert_eq!(
            persist_report("<findings>[]</findings>", "previous", &dir, &home, &Idle).unwrap(),
            0
        );
        let previous = fs::read(dir.join("previous.json")).unwrap();
        for output in [
            "no report",
            "<findings>oops</findings>",
            "<findings>{}</findings>",
            "<findings>[]",
            "<findings>[]</findings><findings>[]</findings>",
        ] {
            assert!(persist_report(output, "new", &dir, &home, &Idle).is_err());
            assert!(!dir.join("new.json").exists());
            assert_eq!(fs::read(dir.join("previous.json")).unwrap(), previous);
        }
        let unwritable = home.join("file-not-directory");
        fs::write(&unwritable, "fixture").unwrap();
        assert!(
            persist_report("<findings>[]</findings>", "new", &unwritable, &home, &Idle).is_err()
        );
        assert!(persist_report("<findings>[]</findings>", "previous", &dir, &home, &Idle).is_err());
        assert_eq!(fs::read(dir.join("previous.json")).unwrap(), previous);
        let saved: serde_json::Value = serde_json::from_slice(&previous).unwrap();
        assert_eq!(
            saved["message"],
            "Scan complete — no cleanup candidates found"
        );
        assert!(saved["created_at"].is_string());
    }

    #[test]
    fn acceptance_8_persist_reclassifies_and_rejects_unknown_duplicate_overlapping_scope() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let path = home.join("repo/target");
        fs::create_dir_all(&path).unwrap();
        fs::write(home.join("repo/Cargo.toml"), "[package]").unwrap();
        let row = serde_json::json!({ "id": "target", "type": "dev_cache", "path": path,
            "size_bytes": 100, "age_days": null, "category": "in_use", "recommendation": "Safe to remove" });
        let emit = |rows: serde_json::Value| format!("<findings>{rows}</findings>");
        let dir = home.join("findings");
        persist_report(
            &emit(serde_json::json!([row.clone()])),
            "valid",
            &dir,
            &home,
            &Idle,
        )
        .unwrap();
        let ledger: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("valid.json")).unwrap()).unwrap();
        assert_eq!(ledger["findings"][0]["category"], "in_use");
        assert!(ledger["findings"][0]["observed_at"].is_string());
        assert!(persist_report(
            &emit(serde_json::json!([row.clone(), row.clone()])),
            "duplicate",
            &dir,
            &home,
            &Idle
        )
        .is_err());
        for category in [serde_json::Value::Null, serde_json::json!("unknown")] {
            let mut invalid = row.clone();
            invalid["category"] = category;
            assert!(persist_report(
                &emit(serde_json::json!([invalid])),
                "unknown",
                &dir,
                &home,
                &Idle
            )
            .is_err());
        }
        let mut outside = row.clone();
        outside["path"] = serde_json::json!(home);
        assert!(persist_report(
            &emit(serde_json::json!([outside])),
            "outside",
            &dir,
            &home,
            &Idle
        )
        .is_err());
        let child = path.join("nested/target");
        fs::create_dir_all(&child).unwrap();
        fs::write(path.join("nested/Cargo.toml"), "[package]").unwrap();
        let mut overlapping = row.clone();
        overlapping["id"] = serde_json::json!("child");
        overlapping["path"] = serde_json::json!(child);
        assert!(persist_report(
            &emit(serde_json::json!([row, overlapping])),
            "overlap",
            &dir,
            &home,
            &Idle
        )
        .is_err());
    }
}

#[cfg(test)]
mod structured_report_tests {
    use super::*;
    use crate::storage_health::inuse::action::ActivityProbe;
    use crate::storage_health::inuse::ProcessRef;

    struct Idle;
    impl ActivityProbe for Idle {
        fn check(&self, _: &Path) -> Result<Vec<ProcessRef>, String> {
            Ok(Vec::new())
        }
    }

    fn home_with(paths: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        for (relative, kind) in paths {
            let path = home.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            match *kind {
                "dir" => fs::create_dir_all(&path).unwrap(),
                "file" => fs::write(&path, "fixture-bytes").unwrap(),
                other => panic!("unknown fixture kind {other}"),
            }
        }
        (temp, home)
    }

    fn finding(id: &str, kind: &str, path: &Path, category: &str) -> ScanFinding {
        ScanFinding {
            id: id.into(),
            finding_type: kind.into(),
            path: path.to_string_lossy().into_owned(),
            size_bytes: 100,
            age_days: None,
            recommendation: classify::category_label(category).into(),
            category: category.into(),
            consequence: Some(format!("{id} consequence")),
        }
    }

    #[test]
    fn prose_without_structured_rows_does_not_invent_findings_or_replace_previous() {
        let (temp, home) = home_with(&[("Library/Caches/App", "dir")]);
        let dir = home.join("findings");
        let previous = finding(
            "app",
            "app_cache",
            &home.join("Library/Caches/App"),
            classify::CAT_SAFE,
        );
        persist_structured_findings(&[previous], "previous", &dir, &home, &Idle).unwrap();
        let before = fs::read(dir.join("previous.json")).unwrap();
        let prose = "Your disk has 6.9 GB of cleanup opportunities across 13 items.";
        assert!(save_declared_storage_report(prose, &[], "new-run", &dir, &home, &Idle).is_err());
        assert!(!dir.join("new-run.json").exists());
        assert_eq!(fs::read(dir.join("previous.json")).unwrap(), before);
        let _ = temp;
    }

    #[test]
    fn tool_payload_is_saved_when_the_assistant_only_wrote_prose() {
        let (temp, home) = home_with(&[
            ("Library/Caches/App", "dir"),
            ("Library/Developer/Xcode/DerivedData/App-abc", "dir"),
            (".cache/huggingface", "dir"),
            ("Downloads/notes.iso", "file"),
        ]);
        let rows = vec![
            finding(
                "cache",
                "app_cache",
                &home.join("Library/Caches/App"),
                classify::CAT_SAFE,
            ),
            finding(
                "xcode",
                "build_artifact",
                &home.join("Library/Developer/Xcode/DerivedData/App-abc"),
                classify::CAT_SAFE,
            ),
            finding(
                "models",
                "app_cache",
                &home.join(".cache/huggingface"),
                classify::CAT_REGENERABLE,
            ),
            finding(
                "download",
                "large_file",
                &home.join("Downloads/notes.iso"),
                classify::CAT_REVIEW,
            ),
        ];
        let payload = embed_scan_findings_payload(&serde_json::to_string(&rows).unwrap());
        let prose = "**Your disk has 6.9 GB of cleanup opportunities across 13 items.**";
        let saved = save_declared_storage_report(
            prose,
            &[payload],
            "run-1",
            &home.join("findings"),
            &home,
            &Idle,
        )
        .unwrap();
        assert_eq!(saved, 4);
        let ledger: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("findings/run-1.json")).unwrap()).unwrap();
        assert_eq!(ledger["validated"], true);
        assert_eq!(ledger["run_id"], "run-1");
        let ids: Vec<_> = ledger["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["cache", "xcode", "models", "download"]);
        assert_eq!(ledger["findings"][2]["category"], classify::CAT_REGENERABLE);
        assert_eq!(ledger["findings"][3]["category"], classify::CAT_REVIEW);
        assert!(prose.contains("6.9"));
        assert!(!ledger.to_string().contains("6.9"));
        let _ = temp;
    }

    #[test]
    fn an_existing_validated_ledger_is_kept_when_the_assistant_omits_or_forges_tags() {
        let (temp, home) = home_with(&[
            ("Library/Caches/App", "dir"),
            ("Library/Caches/Other", "dir"),
        ]);
        let dir = home.join("findings");
        persist_structured_findings(
            &[finding(
                "app",
                "app_cache",
                &home.join("Library/Caches/App"),
                classify::CAT_SAFE,
            )],
            "run-1",
            &dir,
            &home,
            &Idle,
        )
        .unwrap();
        let before = fs::read(dir.join("run-1.json")).unwrap();
        let forged = format!(
            "<findings>[{}]</findings>",
            serde_json::to_string(&finding(
                "other",
                "app_cache",
                &home.join("Library/Caches/Other"),
                classify::CAT_SAFE,
            ))
            .unwrap()
        );
        assert_eq!(
            save_declared_storage_report(&forged, &[], "run-1", &dir, &home, &Idle).unwrap(),
            1
        );
        assert_eq!(fs::read(dir.join("run-1.json")).unwrap(), before);
        let _ = temp;
    }

    #[test]
    fn overlaps_stay_visible_and_in_use_ancestors_are_blocked() {
        let (temp, home) = home_with(&[
            ("proj/package.json", "file"),
            ("proj/node_modules/pkg/package.json", "file"),
            ("proj/node_modules/pkg/node_modules/keep", "file"),
            ("repo/Cargo.toml", "file"),
            ("repo/target/artifact", "file"),
        ]);
        fs::create_dir_all(home.join("proj/node_modules")).unwrap();
        fs::create_dir_all(home.join("proj/node_modules/pkg/node_modules")).unwrap();
        fs::create_dir_all(home.join("repo/target")).unwrap();
        let parent = home.join("proj/node_modules");
        let child = home.join("proj/node_modules/pkg/node_modules");
        let target = home.join("repo/target");
        let rows = [
            finding("parent", "dev_cache", &parent, classify::CAT_SAFE),
            finding("child", "dev_cache", &child, classify::CAT_IN_USE),
            finding("target", "dev_cache", &target, classify::CAT_IN_USE),
        ];
        let saved =
            persist_structured_findings(&rows, "overlap", &home.join("findings"), &home, &Idle)
                .unwrap();
        assert_eq!(saved.saved, 3);
        assert_eq!(saved.blocked, 1);
        assert_eq!(saved.contained, 1);
        let ledger: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("findings/overlap.json")).unwrap()).unwrap();
        let by_id = |id: &str| {
            ledger["findings"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == id)
                .unwrap()
        };
        assert_eq!(by_id("child")["category"], classify::CAT_IN_USE);
        assert_eq!(by_id("child")["consequence"], "child consequence");
        assert_eq!(by_id("child")["contained_in"], "parent");
        assert!(by_id("child").get("blocked_reason").is_none());
        assert_eq!(
            by_id("parent")["blocked_reason"],
            "Contains an in-use or protected item"
        );
        assert_eq!(by_id("parent")["category"], classify::CAT_SAFE);
        assert_eq!(by_id("target")["category"], classify::CAT_IN_USE);
        let _ = temp;
    }

    #[test]
    fn a_bad_row_does_not_drop_the_rows_that_validated() {
        let (temp, home) = home_with(&[("Library/Caches/App", "dir")]);
        let good = finding(
            "app",
            "app_cache",
            &home.join("Library/Caches/App"),
            classify::CAT_SAFE,
        );
        let mut bad = good.clone();
        bad.id = "home".into();
        bad.path = home.to_string_lossy().into_owned();
        let saved = persist_structured_findings(
            &[good, bad],
            "partial",
            &home.join("findings"),
            &home,
            &Idle,
        )
        .unwrap();
        assert_eq!(saved.saved, 1);
        assert_eq!(saved.rejected.len(), 1);
        assert_eq!(saved.rejected[0].id, "home");
        let ledger: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("findings/partial.json")).unwrap()).unwrap();
        assert_eq!(ledger["findings"][0]["id"], "app");
        assert_eq!(ledger["rejected"][0]["id"], "home");
        assert_eq!(ledger["validated"], true);
        assert_eq!(ledger["message"], "Scan complete");
        let bytes = fs::read(home.join("findings/partial.json")).unwrap();
        persist_structured_findings(
            &[finding(
                "other",
                "app_cache",
                &home.join("Library/Caches/App"),
                classify::CAT_SAFE,
            )],
            "partial",
            &home.join("findings"),
            &home,
            &Idle,
        )
        .unwrap();
        assert_eq!(fs::read(home.join("findings/partial.json")).unwrap(), bytes);
        let _ = temp;
    }

    #[test]
    fn an_all_rejected_scan_keeps_reasons_and_is_not_an_empty_success() {
        let (temp, home) = home_with(&[]);
        let dir = home.join("findings");
        let outside = finding(
            "outside",
            "app_cache",
            &home.join("not-a-cache"),
            classify::CAT_SAFE,
        );
        let mut invalid = outside.clone();
        invalid.id = "blank".into();
        invalid.path.clear();
        let saved =
            persist_structured_findings(&[outside, invalid], "all-rejected", &dir, &home, &Idle)
                .unwrap();
        assert_eq!(saved.saved, 0);
        assert_eq!(saved.rejected.len(), 2);
        let reasons: Vec<_> = saved
            .rejected
            .iter()
            .map(|row| (row.id.as_str(), row.reason.as_str()))
            .collect();
        assert!(reasons.iter().any(|(id, _)| *id == "outside"));
        assert!(reasons
            .iter()
            .any(|(id, reason)| *id == "blank" && !reason.is_empty()));
        let ledger: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("all-rejected.json")).unwrap()).unwrap();
        assert_eq!(ledger["validated"], true);
        assert_eq!(ledger["findings"].as_array().unwrap().len(), 0);
        assert_eq!(
            ledger["message"],
            "No actionable cleanup findings — scanned items were rejected."
        );
        assert_ne!(
            ledger["message"],
            "Scan complete — no cleanup candidates found"
        );
        let rejected = ledger["rejected"].as_array().unwrap();
        assert_eq!(rejected.len(), 2);
        assert!(rejected.iter().all(|row| {
            row.get("id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| !id.is_empty())
                && row
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .is_some_and(|reason| !reason.is_empty())
                && row.get("category").is_none()
                && row.get("recommendation").is_none()
        }));
        let bytes = fs::read(dir.join("all-rejected.json")).unwrap();
        let again = persist_structured_findings(
            &[finding(
                "other",
                "app_cache",
                &home.join("Library/Caches/Other"),
                classify::CAT_SAFE,
            )],
            "all-rejected",
            &dir,
            &home,
            &Idle,
        )
        .unwrap();
        assert_eq!(again.saved, 0);
        assert_eq!(again.rejected.len(), 2);
        assert_eq!(fs::read(dir.join("all-rejected.json")).unwrap(), bytes);

        let empty = persist_structured_findings(&[], "empty-scan", &dir, &home, &Idle).unwrap();
        assert_eq!(empty.saved, 0);
        assert!(empty.rejected.is_empty());
        let empty_ledger: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("empty-scan.json")).unwrap()).unwrap();
        assert_eq!(empty_ledger["validated"], true);
        assert_eq!(empty_ledger["findings"].as_array().unwrap().len(), 0);
        assert_eq!(empty_ledger["rejected"].as_array().unwrap().len(), 0);
        assert_eq!(
            empty_ledger["message"],
            "Scan complete — no cleanup candidates found"
        );
        assert_ne!(empty_ledger["message"], ledger["message"]);
        let _ = temp;
    }

    #[test]
    fn an_actioned_ledger_is_not_replaced_by_a_later_scan() {
        let (temp, home) = home_with(&[
            ("Library/Caches/App", "dir"),
            ("Library/Caches/Other", "dir"),
        ]);
        let dir = home.join("findings");
        persist_structured_findings(
            &[finding(
                "app",
                "app_cache",
                &home.join("Library/Caches/App"),
                classify::CAT_SAFE,
            )],
            "run-1",
            &dir,
            &home,
            &Idle,
        )
        .unwrap();
        let mut ledger: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("run-1.json")).unwrap()).unwrap();
        ledger["findings"][0]["action_taken"] = serde_json::json!("trashed");
        ledger["findings"][0]["size_recovered_bytes"] = serde_json::json!(100);
        fs::write(
            dir.join("run-1.json"),
            serde_json::to_vec_pretty(&ledger).unwrap(),
        )
        .unwrap();
        let before = fs::read(dir.join("run-1.json")).unwrap();
        let again = persist_structured_findings(
            &[finding(
                "other",
                "app_cache",
                &home.join("Library/Caches/Other"),
                classify::CAT_SAFE,
            )],
            "run-1",
            &dir,
            &home,
            &Idle,
        )
        .unwrap();
        assert_eq!(again.saved, 1);
        assert_eq!(fs::read(dir.join("run-1.json")).unwrap(), before);
        let ledger: serde_json::Value = serde_json::from_slice(&before).unwrap();
        assert_eq!(ledger["findings"][0]["id"], "app");
        assert_eq!(ledger["findings"][0]["action_taken"], "trashed");
        let _ = temp;
    }

    #[test]
    fn a_second_scan_of_the_same_run_keeps_the_first_validated_ledger() {
        let (temp, home) = home_with(&[
            ("Library/Caches/App", "dir"),
            ("Library/Caches/Other", "dir"),
        ]);
        let dir = home.join("findings");
        let first = persist_structured_findings(
            &[finding(
                "app",
                "app_cache",
                &home.join("Library/Caches/App"),
                classify::CAT_SAFE,
            )],
            "run-1",
            &dir,
            &home,
            &Idle,
        )
        .unwrap();
        let before = fs::read(dir.join("run-1.json")).unwrap();
        let again = persist_structured_findings(
            &[finding(
                "other",
                "app_cache",
                &home.join("Library/Caches/Other"),
                classify::CAT_SAFE,
            )],
            "run-1",
            &dir,
            &home,
            &Idle,
        )
        .unwrap();
        assert_eq!(again, first);
        assert_eq!(fs::read(dir.join("run-1.json")).unwrap(), before);
        let _ = temp;
    }

    #[test]
    fn a_corrupt_ledger_is_not_replaced() {
        let (temp, home) = home_with(&[("Library/Caches/App", "dir")]);
        let dir = home.join("findings");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("run-1.json");
        fs::write(&path, b"{not-a-ledger").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(persist_structured_findings(
            &[finding(
                "app",
                "app_cache",
                &home.join("Library/Caches/App"),
                classify::CAT_SAFE,
            )],
            "run-1",
            &dir,
            &home,
            &Idle,
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let _ = temp;
    }

    #[test]
    fn concurrent_scans_do_not_clobber_a_recorded_action_or_tear_the_first_create() {
        let (temp, home) = home_with(&[
            ("Library/Caches/App", "dir"),
            ("Library/Caches/Other", "dir"),
        ]);
        let dir = home.join("findings");
        persist_structured_findings(
            &[finding(
                "app",
                "app_cache",
                &home.join("Library/Caches/App"),
                classify::CAT_SAFE,
            )],
            "actioned",
            &dir,
            &home,
            &Idle,
        )
        .unwrap();
        let actioned = dir.join("actioned.json");
        let mut ledger: serde_json::Value =
            serde_json::from_slice(&fs::read(&actioned).unwrap()).unwrap();
        ledger["findings"][0]["action_taken"] = serde_json::json!("trashed");
        ledger["findings"][0]["size_recovered_bytes"] = serde_json::json!(100);
        fs::write(&actioned, serde_json::to_vec_pretty(&ledger).unwrap()).unwrap();
        let before = fs::read(&actioned).unwrap();

        let mut joins = Vec::new();
        for n in 0..8 {
            let dir = dir.clone();
            let home = home.clone();
            joins.push(std::thread::spawn(move || {
                let other = home.join("Library/Caches/Other");
                persist_structured_findings(
                    &[finding(
                        &format!("other-{n}"),
                        "app_cache",
                        &other,
                        classify::CAT_SAFE,
                    )],
                    "actioned",
                    &dir,
                    &home,
                    &Idle,
                )
            }));
        }
        for join in joins {
            assert!(join.join().unwrap().is_ok());
        }
        assert_eq!(fs::read(&actioned).unwrap(), before);

        let fresh_dir = dir.clone();
        let fresh_home = home.clone();
        let mut fresh = Vec::new();
        for n in 0..8 {
            let dir = fresh_dir.clone();
            let home = fresh_home.clone();
            fresh.push(std::thread::spawn(move || {
                persist_structured_findings(
                    &[finding(
                        &format!("app-{n}"),
                        "app_cache",
                        &home.join("Library/Caches/App"),
                        classify::CAT_SAFE,
                    )],
                    "fresh",
                    &dir,
                    &home,
                    &Idle,
                )
            }));
        }
        for join in fresh {
            assert!(join.join().unwrap().is_ok());
        }
        let created: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("fresh.json")).unwrap()).unwrap();
        assert_eq!(created["validated"], true);
        assert_eq!(created["run_id"], "fresh");
        let rows = created["findings"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        let id = rows[0]["id"].as_str().unwrap();
        assert!(id.starts_with("app-"), "{id}");
        let _ = temp;
    }

    #[test]
    fn scan_tool_names_are_only_the_unprefixed_and_extension_prefixed_forms() {
        assert!(is_storage_scan_tool("scan_storage_health"));
        assert!(is_storage_scan_tool("storage_health__scan_storage_health"));
        for spoof in [
            "evil__scan_storage_health",
            "developer__scan_storage_health",
            "scan_storage",
            "scan_storage_health_extra",
            "storage_health__other",
            "storage_health__scan_storage_health__extra",
        ] {
            assert!(!is_storage_scan_tool(spoof), "{spoof}");
        }
    }
}
