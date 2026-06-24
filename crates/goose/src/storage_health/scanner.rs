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

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
}

/// Resolve dev scan roots: probe conventional directories first, fall back to $HOME.
fn dev_scan_roots() -> Vec<(PathBuf, u32)> {
    let home = home_dir();
    let conventional = ["dev", "code", "src", "projects", "repos", "workspace"];
    // Also check Documents/GitHub (common for GitHub Desktop users)
    let extra = ["Documents/GitHub"];

    let mut roots: Vec<(PathBuf, u32)> = Vec::new();
    for name in &conventional {
        let p = home.join(name);
        if p.is_dir() {
            roots.push((p, 4));
        }
    }
    for name in &extra {
        let p = home.join(name);
        if p.is_dir() {
            roots.push((p, 4));
        }
    }

    if roots.is_empty() {
        // Fallback: scan $HOME with depth 3, skip irrelevant subdirs
        roots.push((home, 3));
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

/// Scan for cargo target/ directories (with Cargo.toml sibling)
/// and node_modules/ directories (with package.json sibling).
pub fn scan_dev_caches(counter: &mut u32) -> Vec<ScanFinding> {
    let mut findings = Vec::new();
    let roots = dev_scan_roots();
    let home = home_dir();

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

            // When scanning $HOME, skip irrelevant top-level dirs
            if *root == home && should_skip_dev_dir(path, &home) {
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

            // Cargo target/ with Cargo.toml sibling
            if name == "target" {
                if let Some(parent) = path.parent() {
                    if parent.join("Cargo.toml").exists() {
                        let sz = size::dir_size(path);
                        if sz > 10_000_000 {
                            // >10MB to be worth reporting
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
                    }
                }
            }

            // node_modules/ with package.json sibling
            if name == "node_modules" {
                if let Some(parent) = path.parent() {
                    if parent.join("package.json").exists() {
                        let sz = size::dir_size(path);
                        if sz > 10_000_000 {
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
                    }
                }
            }
        }
    }

    // Sort largest first
    findings.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    findings
}

/// Scan app cache directories: ~/Library/Caches, ~/.cache, ~/.npm, ~/.cargo/registry.
pub fn scan_app_caches(counter: &mut u32) -> Vec<ScanFinding> {
    let home = home_dir();
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

    findings.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    findings
}

/// Scan Xcode DerivedData.
pub fn scan_xcode_derived(counter: &mut u32) -> Vec<ScanFinding> {
    let derived_data = home_dir().join("Library/Developer/Xcode/DerivedData");
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

    findings.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    findings
}

/// Scan ~/Downloads for files older than 30 days and >10MB.
pub fn scan_stale_downloads(counter: &mut u32) -> Vec<ScanFinding> {
    let downloads = home_dir().join("Downloads");
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

    findings.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    findings
}

/// Scan ~/Downloads, ~/Desktop, ~/Documents for files >100MB.
pub fn scan_large_user_files(counter: &mut u32) -> Vec<ScanFinding> {
    let home = home_dir();
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

    findings.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

    // These tests mutate the process-global `HOME` env var (the scanners
    // resolve their roots via `home_dir()` reading `$HOME`). Rust runs tests
    // in parallel by default, so without serialization a sibling test can
    // clobber `HOME` mid-scan — making `scan_dev_caches` walk the wrong root
    // and find nothing (issue #462: intermittent "should find cargo target/"
    // failure on CI). `#[serial]` forces them to run one at a time.

    #[test]
    #[serial]
    fn test_dev_cache_finds_target_with_cargo_toml() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("myproject");
        fs::create_dir_all(project.join("target/debug")).unwrap();
        fs::write(project.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        // Write >10MB of data
        let data = vec![0u8; 11_000_000];
        fs::write(project.join("target/debug/big.o"), &data).unwrap();

        std::env::set_var("HOME", tmp.path());
        let mut counter = 0;
        let findings = scan_dev_caches(&mut counter);
        assert!(!findings.is_empty(), "should find cargo target/");
        assert_eq!(findings[0].finding_type, "dev_cache");
        assert!(findings[0].path.contains("target"));
        assert!(findings[0].size_bytes >= 11_000_000);
        std::env::remove_var("HOME");
    }

    #[test]
    #[serial]
    fn test_dev_cache_ignores_target_without_cargo_toml() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("notaproject");
        fs::create_dir_all(project.join("target")).unwrap();
        let data = vec![0u8; 11_000_000];
        fs::write(project.join("target/big.o"), &data).unwrap();
        // No Cargo.toml

        std::env::set_var("HOME", tmp.path());
        let mut counter = 0;
        let findings = scan_dev_caches(&mut counter);
        assert!(
            findings.is_empty(),
            "should not find target/ without Cargo.toml"
        );
        std::env::remove_var("HOME");
    }

    #[test]
    #[serial]
    fn test_stale_downloads_respects_mtime() {
        let tmp = TempDir::new().unwrap();
        let downloads = tmp.path().join("Downloads");
        fs::create_dir_all(&downloads).unwrap();

        // Create a fresh file >10MB — should NOT be found (age < 30 days)
        let data = vec![0u8; 11_000_000];
        fs::write(downloads.join("fresh.zip"), &data).unwrap();

        std::env::set_var("HOME", tmp.path());
        let mut counter = 0;
        let findings = scan_stale_downloads(&mut counter);
        assert!(findings.is_empty(), "fresh file should not be flagged");
        std::env::remove_var("HOME");
    }

    #[test]
    #[serial]
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

        std::env::set_var("HOME", tmp.path());
        let mut counter = 0;
        let findings = scan_large_user_files(&mut counter);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].path.contains("huge.iso"));
        assert!(findings[0].size_bytes >= 101_000_000);
        std::env::remove_var("HOME");
    }

    #[test]
    #[serial]
    fn test_scan_empty_dir() {
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());
        let mut counter = 0;
        assert!(scan_dev_caches(&mut counter).is_empty());
        assert!(scan_app_caches(&mut counter).is_empty());
        assert!(scan_xcode_derived(&mut counter).is_empty());
        assert!(scan_stale_downloads(&mut counter).is_empty());
        assert!(scan_large_user_files(&mut counter).is_empty());
        std::env::remove_var("HOME");
    }
}
