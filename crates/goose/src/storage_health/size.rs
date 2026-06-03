//! Fast recursive directory size calculation.

use std::fs;
use std::path::Path;

/// Calculate the total on-disk allocated size of a directory recursively.
///
/// Uses `blocks() * 512` on macOS/Unix (actual disk allocation) rather than
/// `len()` (logical size). This correctly reports reclaimable space for:
/// - Directories (inode metadata vs content)
/// - Compressed files (APFS transparent compression)
/// - Sparse files
///
/// Returns 0 for nonexistent or inaccessible paths.
pub fn dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    if path.is_file() {
        return allocated_size(path);
    }

    let mut total: u64 = 0;
    let walker = ignore::WalkBuilder::new(path)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .hidden(false)
        .build();

    for entry in walker.flatten() {
        if entry.file_type().is_some_and(|ft| ft.is_file()) {
            if let Ok(meta) = entry.metadata() {
                total += allocated_size_from_meta(&meta);
            }
        }
    }
    total
}

/// On-disk allocated size for a single file.
fn allocated_size(path: &Path) -> u64 {
    path.metadata()
        .map(|m| allocated_size_from_meta(&m))
        .unwrap_or(0)
}

/// Extract allocated size from metadata. Uses blocks*512 on Unix,
/// falls back to logical len() on other platforms.
#[cfg(unix)]
fn allocated_size_from_meta(meta: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    // st_blocks is in 512-byte units on macOS/Linux
    meta.blocks() * 512
}

#[cfg(not(unix))]
fn allocated_size_from_meta(meta: &fs::Metadata) -> u64 {
    meta.len()
}

/// Calculate the age of a path in days (based on mtime).
/// Returns None if metadata is unavailable.
pub fn age_days(path: &Path) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let elapsed = modified.elapsed().ok()?;
    Some(elapsed.as_secs() / 86400)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_dir_size_with_files() {
        let tmp = TempDir::new().unwrap();
        let f1 = tmp.path().join("a.txt");
        let f2 = tmp.path().join("sub");
        fs::create_dir(&f2).unwrap();
        let f3 = f2.join("b.txt");
        fs::write(&f1, "hello").unwrap(); // 5 bytes logical
        fs::write(&f3, "world!!").unwrap(); // 7 bytes logical
        let size = dir_size(tmp.path());
        // Allocated size is in block multiples (typically 4096 on APFS),
        // so two small files allocate at least 2 blocks.
        assert!(size > 0, "dir_size should be non-zero for a dir with files");
        assert!(
            size >= 12,
            "allocated size ({size}) should be >= logical size (12)"
        );
    }

    #[test]
    fn test_dir_size_nonexistent() {
        assert_eq!(dir_size(Path::new("/nonexistent/path/xyz")), 0);
    }

    #[test]
    fn test_dir_size_empty() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(dir_size(tmp.path()), 0);
    }

    #[test]
    fn test_age_days() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("new.txt");
        fs::write(&f, "fresh").unwrap();
        let age = age_days(&f);
        assert!(age.is_some());
        assert_eq!(age.unwrap(), 0); // just created
    }
}
