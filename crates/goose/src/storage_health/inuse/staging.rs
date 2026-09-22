//! Trash only a verified entry staged on the same volume. Native Trash will
//! record the staging path; callers retain the original path in the ledger.
use std::path::{Component, Path};

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn directory(path: &Path) -> Result<std::fs::File, String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let mut parent = std::fs::File::open("/").map_err(|e| e.to_string())?;
    for component in path.components() {
        match component {
            Component::RootDir => continue,
            Component::Normal(name) => {
                let name = CString::new(name.as_bytes()).map_err(|e| e.to_string())?;
                // SAFETY: valid descriptor/name; each ancestor is opened without
                // following links, so an ancestor substitution cannot redirect us.
                let fd = unsafe {
                    libc::openat(
                        parent.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                parent = unsafe { std::fs::File::from_raw_fd(fd) };
            }
            _ => return Err("Non-normal staging path".into()),
        }
    }
    Ok(parent)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn rename_exclusive(
    from: &std::fs::File,
    name: &std::ffi::CStr,
    to: &std::fs::File,
    dest: &std::ffi::CStr,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    // No replacement, including during rollback. Never overwrite a new entry
    // a concurrent writer installed at the original location.
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            from.as_raw_fd(),
            name.as_ptr(),
            to.as_raw_fd(),
            dest.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            from.as_raw_fd(),
            name.as_ptr(),
            to.as_raw_fd(),
            dest.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

/// `before_rename` supplies the deterministic race barrier used by acceptance 6.
/// Production supplies an empty closure. The caller validates scanner scope;
/// this layer independently checks resolved-parent containment at rename time.
/// A final-entry substitution moves only that entry, never a symlink's target.
/// Staged dev/ino mismatch triggers a no-overwrite rollback, never Trash. If a
/// concurrent writer occupies the rollback path, retain the staged entry and
/// report its location. This is not an unconditional no-op: the substituted
/// entry is briefly staged, and hostile same-user directory renames cannot be
/// transactionally excluded by advisory Cargo locks or Unix pathname APIs.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn with_staged<T>(
    home: &Path,
    run: &str,
    id: &str,
    source: &Path,
    expected: &std::fs::Metadata,
    before_rename: impl FnOnce(),
    trash: impl FnOnce(&Path) -> Result<T, String>,
) -> Result<T, String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let parent = source.parent().ok_or("Missing source parent")?;
    let allowed_parent = parent.canonicalize().map_err(|e| e.to_string())?;
    let cursor_root = std::env::temp_dir()
        .canonicalize()
        .ok()
        .map(|p| p.join("cursor-sandbox-cache"));
    if !allowed_parent.starts_with(home)
        && !cursor_root
            .as_ref()
            .is_some_and(|p| allowed_parent.starts_with(p))
    {
        return Err("Resolved parent is outside allowed cleanup roots".into());
    }
    if allowed_parent != parent {
        return Err("Resolved parent changed outside allowed cleanup roots".into());
    }
    let source_dir = directory(parent)?;
    let source_name = CString::new(source.file_name().ok_or("Missing source name")?.as_bytes())
        .map_err(|e| e.to_string())?;
    let safe_name = |s: &str| {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    let mut base = home.to_path_buf();
    for component in [".permagent", "automation", "trash-staging", &safe_name(run)] {
        if component.is_empty() {
            return Err("Missing staging run identity".into());
        }
        base.push(component);
        match std::fs::DirBuilder::new().mode(0o700).create(&base) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.to_string()),
        }
        let opened = directory(&base)?;
        let metadata = opened.metadata().map_err(|e| e.to_string())?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
            return Err(
                "Staging directory must be owned by this user and not writable by others".into(),
            );
        }
    }
    let staging = tempfile::Builder::new()
        .prefix(&format!("{}-", safe_name(id)))
        .tempdir_in(&base)
        .map_err(|e| e.to_string())?;
    let stage_dir = directory(staging.path())?;
    if stage_dir.metadata().map_err(|e| e.to_string())?.dev() != expected.dev() {
        return Err("Staging is on a different volume; cleanup skipped".into());
    }
    let dest = CString::new("item").map_err(|e| e.to_string())?;
    before_rename();
    // Validate the resolved parent again immediately before rename. openat's
    // anchored fd closes the later pathname-symlink window as well.
    if parent.canonicalize().map_err(|e| e.to_string())? != allowed_parent {
        return Err("Resolved parent changed outside allowed cleanup roots".into());
    }
    let current_parent = directory(parent)?.metadata().map_err(|e| e.to_string())?;
    let opened_parent = source_dir.metadata().map_err(|e| e.to_string())?;
    if current_parent.dev() != opened_parent.dev() || current_parent.ino() != opened_parent.ino() {
        return Err("Source parent identity changed".into());
    }
    rename_exclusive(&source_dir, &source_name, &stage_dir, &dest)?;
    // Keep the private directory on failure: TempDir drop must never delete a
    // substituted item or a verified item that native Trash failed to move.
    let staging = staging.keep();
    let staged = staging.join("item");
    let result = (|| {
        let actual = std::fs::symlink_metadata(&staged).map_err(|e| e.to_string())?;
        if actual.dev() != expected.dev()
            || actual.ino() != expected.ino()
            || actual.file_type().is_symlink()
        {
            return Err("Path changed between validation and staging; nothing trashed".into());
        }
        trash(&staged)
    })();
    if let Err(reason) = result {
        if let Err(restore) = rename_exclusive(&stage_dir, &dest, &source_dir, &source_name) {
            return Err(format!(
                "{reason}; restore failed: {restore}; retained at {}",
                staged.display()
            ));
        }
        let _ = std::fs::remove_dir(staging);
        return Err(reason);
    }
    let _ = std::fs::remove_dir(staging);
    result
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn with_staged<T>(
    _: &Path,
    _: &str,
    _: &str,
    _: &Path,
    _: &std::fs::Metadata,
    _: impl FnOnce(),
    _: impl FnOnce(&Path) -> Result<T, String>,
) -> Result<T, String> {
    Err("Verified atomic Trash staging is unsupported on this platform".into())
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;
    #[test]
    fn acceptance_6_final_entry_symlink_swap_is_restored_without_trashing_outside_data() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        std::fs::create_dir(home.join("Downloads")).unwrap();
        let source = home.join("Downloads/item");
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("private");
        std::fs::write(&victim, "outside roots").unwrap();
        std::fs::write(&source, "expected").unwrap();
        let expected = std::fs::symlink_metadata(&source).unwrap();
        let result = with_staged(
            &home,
            "run",
            "id",
            &source,
            &expected,
            || {
                std::fs::rename(&source, home.join("Downloads/saved")).unwrap();
                std::os::unix::fs::symlink(&victim, &source).unwrap();
            },
            |_| -> Result<(), String> { panic!("substitution must not reach Trash") },
        );
        assert!(result
            .unwrap_err()
            .contains("Path changed between validation and staging"));
        assert!(std::fs::symlink_metadata(&source)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(victim).unwrap(), "outside roots");
        assert_eq!(
            std::fs::read_to_string(home.join("Downloads/saved")).unwrap(),
            "expected"
        );
    }

    #[test]
    fn acceptance_6_parent_symlink_swap_cannot_move_outside_entry() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        std::fs::create_dir(home.join("Downloads")).unwrap();
        let source = home.join("Downloads/item");
        std::fs::write(&source, "expected").unwrap();
        let expected = std::fs::symlink_metadata(&source).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("item"), "outside roots").unwrap();
        let result = with_staged(
            &home,
            "run",
            "id",
            &source,
            &expected,
            || {
                std::fs::rename(home.join("Downloads"), home.join("saved")).unwrap();
                std::os::unix::fs::symlink(outside.path(), home.join("Downloads")).unwrap();
            },
            |_| -> Result<(), String> { panic!("parent substitution must not reach Trash") },
        );
        assert!(result.unwrap_err().contains("Resolved parent changed"));
        assert_eq!(
            std::fs::read_to_string(outside.path().join("item")).unwrap(),
            "outside roots"
        );
    }
}
