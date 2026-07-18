//! Atomic, permission-safe persistence for secret material.
//!
//! Every secret that touches disk must go through these helpers. They close
//! the two classic TOCTOU windows of the `fs::write` + `set_permissions`
//! pattern:
//!
//! 1. The file is never observable with permissive modes: content is staged in
//!    a same-directory temp file created `0600` from the first byte and then
//!    atomically renamed over the destination.
//! 2. The parent directory is created `0700` from the start, and — unlike a
//!    create-time-only chmod — the mode is re-enforced on every call so a
//!    pre-existing loose directory gets tightened rather than trusted.

use std::io::Write;
use std::path::Path;

/// Create `dir` (and any missing ancestors) and enforce owner-only (`0700`)
/// permissions on `dir` itself.
///
/// Missing ancestors are created `0700` as well; the modes of pre-existing
/// ancestors (e.g. `~/.permagent`) are deliberately left untouched.
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        use std::os::unix::fs::PermissionsExt;

        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        // `mode` only applies to directories created above. If `dir` already
        // existed with looser permissions, tighten it now.
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }

    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)?;
    }

    Ok(())
}

/// Atomically write `contents` to `path` such that the file is only ever
/// visible with owner-only (`0600`) permissions.
///
/// The data is staged in a temp file in the same directory (created `0600` by
/// `tempfile` on Unix) and renamed into place, so readers observe either the
/// old file or the complete new one — never a partial or world-readable
/// window. Overwriting an existing file also replaces its permissions.
pub fn write_private_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "secret file path has no parent directory: {}",
                path.display()
            ),
        )
    })?;

    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;

    // tempfile already creates 0600 on Unix; enforce explicitly so the
    // guarantee does not hinge on a dependency's default.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }

    tmp.write_all(contents)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn ensure_private_dir_creates_with_0700() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("nested").join("secrets");
        ensure_private_dir(&dir).unwrap();

        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // Created ancestor is private too.
        let parent_mode = std::fs::metadata(root.path().join("nested"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(parent_mode, 0o700);
    }

    #[test]
    #[cfg(unix)]
    fn ensure_private_dir_tightens_existing_loose_dir() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("secrets");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        ensure_private_dir(&dir).unwrap();

        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    #[cfg(unix)]
    fn write_private_file_is_0600_and_roundtrips() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("token.json");
        write_private_file(&path, b"{\"token\":\"s3cret\"}").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"token\":\"s3cret\"}");
    }

    #[test]
    #[cfg(unix)]
    fn write_private_file_replaces_loose_permissions_on_overwrite() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("token.json");
        // Simulate a pre-existing world-readable secret from an old version.
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_private_file(&path, b"new").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn write_private_file_overwrites_atomically() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("token.json");
        write_private_file(&path, b"first").unwrap();
        write_private_file(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        // No stray temp files left behind.
        let entries: Vec<_> = std::fs::read_dir(root.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }
}
