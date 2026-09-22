//! Fail-closed action-time checks. Scan-time observations are not authority.
use super::{InUseConfig, ProcessProbe, ProcessRef};
use crate::storage_health::{classify, safety};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A failed or incomplete observation must never mean idle.
pub trait ActivityProbe: Send + Sync {
    fn check(&self, path: &Path) -> Result<Vec<ProcessRef>, String>;
}

pub struct ActionProbe;
impl ActivityProbe for ActionProbe {
    fn check(&self, path: &Path) -> Result<Vec<ProcessRef>, String> {
        // Temporary files avoid pipe-buffer deadlocks while enforcing a deadline.
        let stdout = tempfile::tempfile().map_err(|e| e.to_string())?;
        let stderr = tempfile::tempfile().map_err(|e| e.to_string())?;
        let mut command = Command::new("lsof");
        command.args(["-n", "-P", "-F", "pcn"]);
        if path.is_dir() {
            command.arg("+D");
        } else {
            command.arg("--");
        }
        let mut child = command
            .arg(path)
            .stdout(Stdio::from(stdout.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::from(stderr.try_clone().map_err(|e| e.to_string())?))
            .spawn()
            .map_err(|e| format!("Activity check failed: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                outcome => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("Activity check timed out or failed: {outcome:?}"));
                }
            }
        };
        use std::io::{Read, Seek};
        let read = |mut file: std::fs::File| -> Result<String, String> {
            file.rewind().map_err(|e| e.to_string())?;
            let mut text = String::new();
            file.take(1_048_577)
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            if text.len() > 1_048_576 {
                return Err("Activity information is incomplete".into());
            }
            Ok(text)
        };
        let output = read(stdout)?;
        let warnings = read(stderr)?;
        if !warnings.trim().is_empty() || !matches!(status.code(), Some(0 | 1)) {
            return Err("Activity information is incomplete or permission was denied".into());
        }
        let processes = super::parse_lsof_fields(&output);
        if (!output.trim().is_empty() && processes.is_empty())
            || (status.success() && output.is_empty())
        {
            return Err("Activity information is incomplete".into());
        }
        Ok(processes)
    }
}

struct Observed(Vec<ProcessRef>);
impl ProcessProbe for Observed {
    fn processes_in(&self, _: &Path) -> Vec<ProcessRef> {
        self.0.clone()
    }
    fn holders_of(&self, _: &Path) -> Vec<ProcessRef> {
        self.0.clone()
    }
}

/// Restrict authority to the scanner's cleanup shapes, never a model's label.
/// Reject aliases and symlink components before matching scope.
pub fn validate_path(kind: &str, path: &Path, home: &Path) -> Result<(), String> {
    if !home.is_absolute() {
        return Err("Cleanup home is unavailable".into());
    }
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err("Cleanup requires an absolute, normalized path".into());
    }
    let mut prefix = PathBuf::new();
    for part in path.components() {
        prefix.push(part);
        let meta =
            std::fs::symlink_metadata(&prefix).map_err(|e| format!("Path unavailable: {e}"))?;
        if meta.file_type().is_symlink() {
            return Err("Symlink paths are excluded".into());
        }
    }
    let cursor_root = std::env::temp_dir()
        .canonicalize()
        .ok()
        .map(|p| p.join("cursor-sandbox-cache"));
    let cursor_target = kind == "dev_cache"
        && path.file_name().is_some_and(|n| n == "cargo-target")
        && path
            .parent()
            .and_then(Path::parent)
            .is_some_and(|p| cursor_root.as_deref() == Some(p));
    let protected = [
        ".ssh",
        ".aws",
        ".gcp",
        ".gnupg",
        ".permagent",
        ".Trash",
        "Keychains",
        "Mobile Documents",
    ];
    if path.components().any(|c| {
        protected
            .iter()
            .any(|p| c.as_os_str().to_string_lossy().eq_ignore_ascii_case(p))
    }) || (!cursor_target && safety::is_excluded(path.strip_prefix(home).unwrap_or(path)))
        || (path.is_file() && safety::is_icloud_evicted(path))
    {
        return Err("Sensitive, system, or iCloud-not-downloaded path is excluded".into());
    }
    let in_home = path.starts_with(home) && path != home;
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let scoped = match kind {
        "dev_cache" => {
            cursor_target
                || (in_home
                    && ((name == "target"
                        && path
                            .parent()
                            .is_some_and(|p| p.join("Cargo.toml").is_file()))
                        || (name == "node_modules"
                            && path
                                .parent()
                                .is_some_and(|p| p.join("package.json").is_file()))
                        || path
                            .parent()
                            .and_then(Path::file_name)
                            .is_some_and(|n| n == ".shared-target")))
        }
        "app_cache" => {
            in_home
                && [
                    "Library/Caches",
                    ".cache",
                    ".npm",
                    ".cargo/registry",
                    ".cargo/git",
                    ".rustup",
                ]
                .iter()
                .any(|root| path.starts_with(home.join(root)))
        }
        "build_artifact" => {
            in_home
                && path.starts_with(home.join("Library/Developer/Xcode/DerivedData"))
                && path != home.join("Library/Developer/Xcode/DerivedData")
        }
        "stale_file" => in_home && path.is_file() && path.starts_with(home.join("Downloads")),
        "large_file" => {
            in_home
                && path.is_file()
                && ["Downloads", "Desktop", "Documents"]
                    .iter()
                    .any(|root| path.starts_with(home.join(root)))
        }
        _ => false,
    };
    if !scoped {
        return Err("Path is outside the scanner's cleanup scope".into());
    }
    Ok(())
}

pub fn current_classification(
    kind: &str,
    path: &Path,
    bytes: u64,
    probe: &dyn ActivityProbe,
) -> Result<classify::Classification, String> {
    let processes = probe.check(path)?;
    // All types are checked, including files: an open download is not idle.
    if !processes.is_empty() {
        return Ok(classify::Classification {
            category: classify::CAT_IN_USE,
            recommendation: classify::category_label(classify::CAT_IN_USE).into(),
            consequence: Some(super::summarize_processes(&processes)),
        });
    }
    // Scan-time mtimes are a conservative hint, not proof that the process
    // is still active. The action path has just performed a fresh recursive
    // process check; Cargo additionally requires a coordinating lock.
    let cfg = InUseConfig {
        recent_mtime_minutes: 0,
        ..InUseConfig::default()
    };
    Ok(classify::classify(
        kind,
        path,
        bytes,
        &cfg,
        &Observed(processes),
    ))
}

/// The guard lives until the native Trash operation returns.
pub trait CleanupLock: Send {}
impl CleanupLock for Vec<std::fs::File> {}
pub trait LockProvider: Send + Sync {
    fn acquire(&self, path: &Path) -> Result<Box<dyn CleanupLock>, String>;
}
/// Release gate required by ASTRA_RULING_ST1_CARGO: enable only after real
/// Cargo acceptance 4.1–4.5 pass for the declared configuration. Unit-process
/// flock fixtures do not establish that Cargo itself follows the protocol.
/// Demonstrated 2026-09-22 against a real cargo 1.97.1 build on a throwaway
/// target (`real_cargo_lock_demo` below, driven from a shell): (1) with a build
/// running, acquire was refused "Profile in use: .cargo-build-lock is held" and
/// succeeded after it finished; (2) with cleanup holding the locks, an arriving
/// `cargo build` printed "Blocking waiting for file lock on build directory",
/// waited 17 s for the release and then finished; (3) all three lock-file
/// inodes were identical before and after; (4) a `--profile custom` build
/// during the hold completed in 0.19 s into its own directory with its own
/// locks. Re-run the demonstration and record it here before changing the
/// lock protocol or the cargo version.
pub const CARGO_PROFILE_RELEASE_VERIFIED: bool = true;

pub const CARGO_COORDINATION_FILES: [&str; 3] =
    [".cargo-build-lock", ".cargo-lock", ".cargo-artifact-lock"];

/// Cargo rust-1.97.0 layout.rs executable order: build lock, shared
/// compatibility lock, then artifact lock (when required). Cleanup takes all
/// three exclusively using LOCK_EX | LOCK_NB, never creates/replaces a coordination file, and never
/// removes its profile. A separate build-dir without this complete colocated
/// layout is unsupported. New-layout/fine-grain configurations need real
/// Cargo coverage before the release gate can be enabled.
/// Verified against rust-1.97.0 src/cargo/core/compiler/layout.rs (new) and
/// src/cargo/util/flock.rs (open_* and try_acquire): Cargo uses File::try_lock /
/// try_lock_shared (Unix flock), then blocks; it tolerates ENOTSUP/ENOSYS and
/// skips NFS. Cleanup never tolerates those failures and never blocks.
/// All three existing colocated files are deliberately required even though
/// Cargo only creates the artifact lock when must_take_artifact_dir_lock is
/// true. Missing locks and separate/experimental configurations stay excluded.
pub struct CargoLocks;
impl LockProvider for CargoLocks {
    fn acquire(&self, profile: &Path) -> Result<Box<dyn CleanupLock>, String> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
            if profile.canonicalize().map_err(|e| e.to_string())? != profile {
                return Err("Unsupported layout: aliased Cargo profile".into());
            }
            let directory = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
                .open(profile)
                .map_err(|e| format!("Unsupported layout: {e}"))?;
            // Cargo can skip locking on NFS or unsupported filesystems. Accept
            // only explicitly local filesystem types, before calling flock.
            let mut fs = std::mem::MaybeUninit::<libc::statfs>::uninit();
            if unsafe { libc::fstatfs(directory.as_raw_fd(), fs.as_mut_ptr()) } != 0 {
                return Err("Unsupported layout: filesystem exclusion cannot be verified".into());
            }
            let fs = unsafe { fs.assume_init() };
            #[cfg(target_os = "macos")]
            let supported = matches!(
                unsafe { std::ffi::CStr::from_ptr(fs.f_fstypename.as_ptr()) }.to_bytes(),
                b"apfs" | b"hfs"
            );
            #[cfg(target_os = "linux")]
            let supported = matches!(
                fs.f_type as u64,
                0xEF53 | 0x58465342 | 0x9123683E | 0x01021994
            );
            if !supported {
                return Err(
                    "Unsupported layout: Cargo exclusion is unverified on this filesystem".into(),
                );
            }
            let mut locks = Vec::new();
            let mut identities = Vec::new();
            for name in CARGO_COORDINATION_FILES {
                let path = profile.join(name);
                let before = std::fs::symlink_metadata(&path)
                    .map_err(|e| format!("Unsupported layout: required {name} unavailable: {e}"))?;
                if !before.is_file() || before.file_type().is_symlink() || before.nlink() != 1 {
                    return Err(format!(
                        "Unsupported layout: {name} must be a unique regular file"
                    ));
                }
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&path)
                    .map_err(|e| format!("Cannot open Cargo profile lock {name}: {e}"))?;
                let opened = file.metadata().map_err(|e| e.to_string())?;
                if opened.dev() != before.dev() || opened.ino() != before.ino() {
                    return Err("Unsupported layout: Cargo lock identity changed".into());
                }
                // File remains open in the guard; no blocking acquisition.
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::WouldBlock {
                        return Err(format!("Profile in use: {name} is held"));
                    }
                    return Err(format!("Cannot verify Cargo profile lock {name}: {error}"));
                }
                identities.push((path, opened));
                locks.push(file);
            }
            for (path, opened) in identities {
                let current = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
                if current.dev() != opened.dev() || current.ino() != opened.ino() {
                    return Err("Unsupported layout: Cargo coordination file changed".into());
                }
            }
            locks.push(directory);
            Ok(Box::new(locks))
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = profile;
            Err(
                "Unsupported layout: Cargo directory locking is unsupported on this platform"
                    .into(),
            )
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct CargoProfile {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub reason: Option<String>,
}

/// Snapshot existing profiles only. Neither discovery nor cleanup visits root
/// outputs (doc, tmp, package, timings, metadata) or creates a new profile.
pub fn cargo_profiles(target: &Path) -> Result<Vec<CargoProfile>, String> {
    let mut result = Vec::new();
    for entry in std::fs::read_dir(target).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let name = entry.file_name();
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir()
            || ["doc", "tmp", "package", "cargo-timings", ".metabuild"]
                .iter()
                .any(|n| name == *n)
        {
            continue;
        }
        let marker = |p: &Path| CARGO_COORDINATION_FILES.iter().any(|n| p.join(n).exists());
        let mut paths = Vec::new();
        if marker(&path) || name == "debug" || name == "release" {
            paths.push(path.clone());
        } else {
            for child in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
                let child = child.map_err(|e| e.to_string())?;
                if child.file_type().map_err(|e| e.to_string())?.is_dir() && marker(&child.path()) {
                    paths.push(child.path());
                }
            }
            if paths.is_empty() {
                paths.push(path);
            }
        }
        for profile in paths {
            let measured = profile_contents(&profile).and_then(|items| {
                items.iter().try_fold(0u64, |total, path| {
                    total
                        .checked_add(measured_bytes(path)?)
                        .ok_or_else(|| "Byte estimate overflow".to_string())
                })
            });
            let (size_bytes, reason) = match measured {
                Ok(bytes) => (bytes, None),
                Err(reason) => (
                    crate::storage_health::size::dir_size(&profile),
                    Some(format!("Unsupported layout: {reason}")),
                ),
            };
            result.push(CargoProfile {
                path: profile,
                size_bytes,
                reason,
            });
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

/// Measure the entries actually moved, without following symlinks or silently
/// treating unreadable descendants as zero. Bytes remain an allocation estimate
/// (hard links and filesystem clones mean this is not space freed).
pub fn measured_bytes(path: &Path) -> Result<u64, String> {
    let mut pending = vec![path.to_path_buf()];
    let mut bytes = 0u64;
    while let Some(entry) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&entry).map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            for child in std::fs::read_dir(&entry).map_err(|e| e.to_string())? {
                pending.push(child.map_err(|e| e.to_string())?.path());
            }
        } else {
            #[cfg(unix)]
            let allocated = {
                use std::os::unix::fs::MetadataExt;
                metadata
                    .blocks()
                    .checked_mul(512)
                    .ok_or("Byte estimate overflow")?
            };
            #[cfg(not(unix))]
            let allocated = metadata.len();
            bytes = bytes
                .checked_add(allocated)
                .ok_or("Byte estimate overflow")?;
        }
    }
    Ok(bytes)
}

/// Only the profile's regenerable outputs, never its directory or locks.
pub fn profile_contents(profile: &Path) -> Result<Vec<PathBuf>, String> {
    for name in CARGO_COORDINATION_FILES {
        let m = std::fs::symlink_metadata(profile.join(name))
            .map_err(|_| format!("Unsupported layout: missing {name}"))?;
        if !m.is_file() || m.file_type().is_symlink() {
            return Err(format!("Unsupported layout: invalid {name}"));
        }
    }
    let mut result = Vec::new();
    for entry in std::fs::read_dir(profile).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Preserve every coordination file, including unknown future locks.
        if name.starts_with(".cargo") || name.ends_with(".lock") {
            continue;
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink()
            || (!kind.is_file()
                && !(kind.is_dir()
                    && (["build", "deps", "incremental", ".fingerprint", "examples"]
                        .contains(&name.as_ref())
                        || name.ends_with(".dSYM"))))
        {
            return Err(format!(
                "Unsupported layout: unrecognized profile output {name}"
            ));
        }
        if name.starts_with('.') && name != ".fingerprint" {
            return Err(format!(
                "Unsupported layout: unknown profile metadata {name}"
            ));
        }
        result.push(entry.path());
    }
    result.sort();
    Ok(result)
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod profile_tests {
    use super::*;
    use std::io::{BufRead, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;

    fn profile(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.join("deps")).unwrap();
        for lock in CARGO_COORDINATION_FILES {
            std::fs::write(path.join(lock), "").unwrap();
        }
        std::fs::write(path.join("deps/artifact"), "compiled bytes").unwrap();
        path
    }

    /// Spawned by current_exe only; normal discovery is a no-op. This models
    /// Cargo's OS locks, NOT Cargo's build behavior or configuration handling.
    #[test]
    fn lock_child() {
        let Some(path) = std::env::var_os("ST1_PROFILE_LOCK_CHILD") else {
            return;
        };
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        println!("\nready");
        std::io::stdout().flush().unwrap();
        let mut signal = String::new();
        std::io::stdin().read_line(&mut signal).unwrap();
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            assert_eq!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::WouldBlock
            );
            println!("blocked");
            std::io::stdout().flush().unwrap();
            assert_eq!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) }, 0);
        }
        if let Some(output) = std::env::var_os("ST1_PROFILE_CHILD_OUTPUT") {
            std::fs::write(output, "child wrote while holding the lock").unwrap();
        }
        println!("held");
        std::io::stdout().flush().unwrap();
        // Hold until stdin closes, or the parent explicitly releases us.
        signal.clear();
        std::io::stdin().read_line(&mut signal).unwrap();
        drop(file);
    }

    struct ChildLock {
        child: std::process::Child,
        lines: std::sync::mpsc::Receiver<String>,
    }
    impl ChildLock {
        fn start(path: &Path) -> Self {
            Self::writer(path, None)
        }
        fn writer(path: &Path, output: Option<&Path>) -> Self {
            let mut command = Command::new(std::env::current_exe().unwrap());
            if let Some(output) = output {
                command.env("ST1_PROFILE_CHILD_OUTPUT", output);
            }
            let mut child = command
                .args([
                    "--exact",
                    "storage_health::inuse::action::profile_tests::lock_child",
                    "--nocapture",
                ])
                .env("ST1_PROFILE_LOCK_CHILD", path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let output = child.stdout.take().unwrap();
            let (send, lines) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(output).lines() {
                    if send.send(line.unwrap()).is_err() {
                        break;
                    }
                }
            });
            let process = Self { child, lines };
            process.expect("ready");
            process
        }
        fn signal(&mut self) {
            self.child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(b"go\n")
                .unwrap();
        }
        fn expect(&self, expected: &str) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let line = self
                    .lines
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .unwrap();
                if line == expected {
                    break;
                }
            }
        }
        fn kill(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        fn release(&mut self) {
            self.child.stdin.take();
        }
    }
    impl Drop for ChildLock {
        fn drop(&mut self) {
            self.kill();
        }
    }

    #[test]
    fn acceptance_4_case_1_child_wins_every_required_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let path = profile(&root, "debug");
        for name in CARGO_COORDINATION_FILES {
            let mut child = ChildLock::start(&path.join(name));
            child.signal();
            child.expect("held");
            assert!(CargoLocks
                .acquire(&path)
                .err()
                .unwrap()
                .starts_with("Profile in use:"));
            assert_eq!(
                std::fs::read(path.join("deps/artifact")).unwrap(),
                b"compiled bytes"
            );
            child.kill();
            assert!(CargoLocks.acquire(&path).is_ok());
        }
    }

    #[test]
    fn acceptance_4_cases_2_and_3_cleanup_wins_waiter_and_new_arrival_share_original_inode() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let path = profile(&root, "release");
        let lock = path.join(".cargo-build-lock");
        let before = std::fs::metadata(&lock).unwrap();
        let guard = CargoLocks.acquire(&path).unwrap();
        let output = path.join("compiled-after-cleanup");
        let mut waiter = ChildLock::writer(&lock, Some(&output));
        waiter.signal();
        waiter.expect("blocked");
        assert!(!output.exists());
        // A cleanup mutation is permitted while the contender demonstrably
        // cannot hold the same-profile lock. The original lock is untouched.
        std::fs::rename(path.join("deps"), root.join("fixture-trash")).unwrap();
        drop(guard);
        waiter.expect("held");
        assert_eq!(
            std::fs::read_to_string(output).unwrap(),
            "child wrote while holding the lock"
        );
        let mut arrival = ChildLock::start(&lock);
        arrival.signal();
        arrival.expect("blocked");
        waiter.release();
        arrival.expect("held");
        assert!(CargoLocks.acquire(&path).is_err());
        arrival.kill();
        assert!(CargoLocks.acquire(&path).is_ok());
        let after = std::fs::metadata(lock).unwrap();
        assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    }

    #[test]
    fn acceptance_2_and_4_case_4_snapshot_preserves_new_custom_profile_and_containers() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        for name in [
            "debug",
            "release",
            "custom-existing",
            "aarch64-apple-darwin/custom-cross",
        ] {
            profile(&root, name);
        }
        std::fs::create_dir(root.join("doc")).unwrap();
        std::fs::write(root.join(".rustc_info.json"), "root metadata").unwrap();
        let snapshot = cargo_profiles(&root).unwrap();
        assert_eq!(snapshot.len(), 4);
        assert!(snapshot.iter().all(|p| p.reason.is_none()));
        let new = root.join("custom-new");
        let mut new_profile_child = None;
        for (index, p) in snapshot.iter().enumerate() {
            let guard = CargoLocks.acquire(&p.path).unwrap();
            if index == 0 {
                // The first cleanup guard is held before a new profile starts.
                profile(&root, "custom-new");
                let mut child = ChildLock::start(&new.join(".cargo-build-lock"));
                child.signal();
                child.expect("held");
                new_profile_child = Some(child);
            }
            let identities: Vec<_> = CARGO_COORDINATION_FILES
                .iter()
                .map(|n| {
                    let m = std::fs::metadata(p.path.join(n)).unwrap();
                    (m.dev(), m.ino())
                })
                .collect();
            for (item, source) in profile_contents(&p.path).unwrap().iter().enumerate() {
                std::fs::rename(source, root.join(format!("trash-{index}-{item}"))).unwrap();
            }
            drop(guard);
            for (name, identity) in CARGO_COORDINATION_FILES.iter().zip(identities) {
                let m = std::fs::metadata(p.path.join(name)).unwrap();
                assert_eq!((m.dev(), m.ino()), identity);
            }
            assert!(p.path.is_dir());
        }
        assert!(new_profile_child.is_some());
        assert!(new.join("deps/artifact").exists());
        assert!(root.join("aarch64-apple-darwin").is_dir());
        assert!(root.join("doc").is_dir());
        assert!(root.join(".rustc_info.json").exists());
    }

    #[test]
    fn acceptance_4_case_5_missing_or_symlink_lock_and_whole_tree_are_unsupported() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        assert!(CargoLocks
            .acquire(&root)
            .err()
            .unwrap()
            .starts_with("Unsupported layout:"));
        let path = profile(&root, "debug");
        std::fs::remove_file(path.join(".cargo-artifact-lock")).unwrap();
        std::os::unix::fs::symlink(path.join(".cargo-lock"), path.join(".cargo-artifact-lock"))
            .unwrap();
        assert!(CargoLocks
            .acquire(&path)
            .err()
            .unwrap()
            .starts_with("Unsupported layout:"));
        assert!(path.join("deps/artifact").exists());
    }
}

/// Real-Cargo acceptance for the per-profile cleanup (ASTRA_RULING_ST1_CARGO,
/// test 4). Not hermetic: driven from a shell that runs a real `cargo build`
/// against a throwaway target. Run with:
/// `ST1_DEMO_PROFILE=<target>/debug ST1_DEMO_EXPECT=busy|free \
///  cargo test -p permagent --lib real_cargo_lock_demo -- --ignored --nocapture`
#[cfg(test)]
mod real_cargo_demo {
    use super::{CargoLocks, LockProvider};
    use std::path::PathBuf;

    #[test]
    #[ignore = "needs a real cargo build racing it; see ASTRA_RULING_ST1_CARGO test 4"]
    fn real_cargo_lock_demo() {
        let profile = PathBuf::from(std::env::var("ST1_DEMO_PROFILE").expect("ST1_DEMO_PROFILE"));
        let expect = std::env::var("ST1_DEMO_EXPECT").expect("ST1_DEMO_EXPECT");
        let hold_ms: u64 = std::env::var("ST1_DEMO_HOLD_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let result = CargoLocks.acquire(&profile);
        match expect.as_str() {
            "busy" => {
                let err = result
                    .err()
                    .expect("cargo holds the profile locks: acquire must fail");
                println!("DEMO busy-as-expected: {err}");
            }
            "free" => {
                let guard = result.expect("no cargo running: acquire must succeed");
                println!("DEMO acquired; holding for {hold_ms} ms");
                std::thread::sleep(std::time::Duration::from_millis(hold_ms));
                drop(guard);
                println!("DEMO released");
            }
            other => panic!("unknown ST1_DEMO_EXPECT {other}"),
        }
    }
}
