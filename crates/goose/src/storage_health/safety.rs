//! Path safety checks for the storage health scanner.
//!
//! Determines which paths should be excluded from scanning and which findings
//! should be tagged as "safe to remove" vs "review before removing".

use std::path::Path;

/// Directories that must never be scanned or included in findings.
const NEVER_SCAN: &[&str] = &[
    "/.permagent/",
    "/Library/Mobile Documents/",
    "/.Trash/",
    "/.ssh/",
    "/.aws/",
    "/.gcp/",
    "/.gnupg/",
    "/Library/Keychains/",
];

/// Filename patterns that indicate sensitive content — never include.
const SENSITIVE_NAMES: &[&str] = &[
    ".env",
    "credentials",
    "id_rsa",
    "id_ed25519",
    ".key",
    ".pem",
];

/// Top-level $HOME subdirectories to skip during dev cache scanning.
/// These never contain cargo target/ or node_modules/ worth reporting.
pub const DEV_SCAN_SKIP_DIRS: &[&str] = &[
    "Pictures",
    "Music",
    "Movies",
    "Public",
    "Library",
    ".Trash",
    "Applications",
    "Sites",
    "Downloads", // handled by stale_downloads scanner
];

/// Returns true if the path must be excluded from scanning entirely.
pub fn is_excluded(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    // Never-scan directory prefixes
    for blocked in NEVER_SCAN {
        if path_str.contains(blocked) {
            return true;
        }
    }

    // /Volumes/, /System/, /private/
    if path_str.starts_with("/Volumes/")
        || path_str.starts_with("/System/")
        || path_str.starts_with("/private/")
    {
        return true;
    }

    // Sensitive filenames
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let name_lower = name.to_lowercase();
        for pattern in SENSITIVE_NAMES {
            if name_lower.contains(pattern) {
                return true;
            }
        }
    }

    false
}

/// Check if a file is an iCloud-evicted stub with no local content.
///
/// An evicted file still reports its full logical size via `metadata.len()`
/// (e.g. 462 MB), but has zero disk-allocated blocks because the actual
/// content lives only in iCloud. Trashing such a file would either fail or
/// silently trigger a download — neither is the right cleanup behavior.
///
/// Detection: `blocks() == 0 && len() > 0` on macOS. This matches the
/// NSURL `fileAllocatedSize == 0` signal measured in the Step 1 experiment.
/// On non-macOS, this always returns false.
#[cfg(target_os = "macos")]
pub fn is_icloud_evicted(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    if let Ok(meta) = std::fs::metadata(path) {
        // A file with logical size > 0 but zero allocated 512-byte blocks
        // is an iCloud stub whose content is not locally present.
        meta.len() > 0 && meta.blocks() == 0
    } else {
        false
    }
}

#[cfg(not(target_os = "macos"))]
pub fn is_icloud_evicted(_path: &Path) -> bool {
    false
}

/// Determine the recommendation for a finding based on its type.
pub fn recommendation_for(finding_type: &str) -> &'static str {
    match finding_type {
        "dev_cache" | "app_cache" | "build_artifact" | "stale_file" => "Safe to remove",
        _ => "Review before removing",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_excludes_ssh() {
        assert!(is_excluded(Path::new("/Users/jesse/.ssh/id_rsa")));
    }

    #[test]
    fn test_excludes_permagent() {
        assert!(is_excluded(Path::new(
            "/Users/jesse/.permagent/brain/memory.db"
        )));
    }

    #[test]
    fn test_excludes_icloud() {
        assert!(is_excluded(Path::new(
            "/Users/jesse/Library/Mobile Documents/com~apple~CloudDocs/file.txt"
        )));
    }

    #[test]
    fn test_excludes_sensitive_filenames() {
        assert!(is_excluded(Path::new("/tmp/.env")));
        assert!(is_excluded(Path::new("/tmp/credentials.json")));
        assert!(is_excluded(Path::new("/tmp/server.key")));
        assert!(is_excluded(Path::new("/tmp/cert.pem")));
    }

    #[test]
    fn test_allows_normal_paths() {
        assert!(!is_excluded(Path::new(
            "/Users/jesse/dev/project/src/main.rs"
        )));
        assert!(!is_excluded(Path::new("/Users/jesse/Downloads/report.pdf")));
    }

    #[test]
    fn test_excludes_system_paths() {
        assert!(is_excluded(Path::new("/System/Library/something")));
        assert!(is_excluded(Path::new("/private/var/folders/tmp")));
        assert!(is_excluded(Path::new("/Volumes/External/backup")));
    }

    #[test]
    fn test_recommendation_safe_types() {
        assert_eq!(recommendation_for("dev_cache"), "Safe to remove");
        assert_eq!(recommendation_for("app_cache"), "Safe to remove");
        assert_eq!(recommendation_for("build_artifact"), "Safe to remove");
        assert_eq!(recommendation_for("stale_file"), "Safe to remove");
    }

    #[test]
    fn test_recommendation_review_types() {
        assert_eq!(recommendation_for("large_file"), "Review before removing");
        assert_eq!(recommendation_for("unknown"), "Review before removing");
    }
}
