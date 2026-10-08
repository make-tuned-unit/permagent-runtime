//! System-audio capture for meeting transcripts — the OTHER side of the call.
//!
//! The microphone hears only the user. A meeting transcript needs the people on
//! the far end, which on macOS means ScreenCaptureKit. The capture itself lives
//! in the `permagent-audiocap` Swift sidecar (see `audiocap/main.swift` for why
//! it is not inline Rust); this module owns its lifecycle and turns its stdout
//! into Tauri events.
//!
//! Contract with the sidecar:
//!   stdout — one absolute WAV path per completed chunk, 16 kHz mono 16-bit
//!   stderr — `READY …` once capturing, `ERROR <kind> <detail>` on failure
//!   exit 2 — Screen Recording permission missing (distinct from a crash)
//!   SIGTERM — flush the partial chunk, then exit
//!
//! Each chunk path is emitted as `system_audio_chunk` so the renderer can POST
//! it to `/api/dictation/transcribe` while the meeting is still running, rather
//! than transcribing an hour of audio after the fact.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tauri::{AppHandle, Emitter, Manager};

const SCOPED_AUDIO_PROTOCOL: &[u8] = b"PERMAGENT_AUDIOCAP_SCOPED_SOURCE_V1";

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemAudioApplication {
    bundle_id: String,
    name: String,
}

fn valid_bundle_id(value: &str) -> bool {
    value.len() <= 255
        && value.contains('.')
        && value.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn validate_source_scope(requested: Option<&str>, own_bundle_id: &str) -> Result<(), String> {
    if let Some(bundle_id) = requested {
        if !valid_bundle_id(bundle_id) {
            return Err("invalid system audio application bundle identifier".into());
        }
        if bundle_id == own_bundle_id {
            return Err("Permagent itself cannot be the call audio source".into());
        }
    }
    Ok(())
}

/// NSWorkspace enumerates running regular applications without ScreenCaptureKit
/// or capture permission. Do not spawn the helper just to populate a picker.
#[tauri::command]
pub async fn list_system_audio_apps(app: AppHandle) -> Result<Vec<SystemAudioApplication>, String> {
    #[cfg(target_os = "macos")]
    {
        let own_bundle_id = app.config().identifier.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let result = list_running_audio_applications(&own_bundle_id);
            let _ = sender.send(result);
        })
        .map_err(|e| e.to_string())?;
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "application list did not respond".to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(Vec::new())
    }
}

#[cfg(target_os = "macos")]
fn list_running_audio_applications(
    own_bundle_id: &str,
) -> Result<Vec<SystemAudioApplication>, String> {
    use objc2::{
        msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::{NSArray, NSString};
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
            let class = AnyClass::get(c"NSWorkspace").ok_or("NSWorkspace is unavailable")?;
            let workspace: *mut AnyObject = msg_send![class, sharedWorkspace];
            if workspace.is_null() {
                return Err("NSWorkspace is unavailable");
            }
            let applications: Retained<NSArray<AnyObject>> =
                msg_send![workspace, runningApplications];
            let mut choices = Vec::new();
            let mut bundles = BTreeSet::new();
            for application in applications.iter() {
                let policy: isize = msg_send![&*application, activationPolicy];
                let terminated: bool = msg_send![&*application, isTerminated];
                // NSApplicationActivationPolicyRegular == 0. Accessory/background
                // processes have no useful picker identity and may be OS services.
                if policy != 0 || terminated {
                    continue;
                }
                let bundle: Option<Retained<NSString>> = msg_send![&*application, bundleIdentifier];
                let Some(bundle) = bundle else {
                    continue;
                };
                let bundle_id = bundle.to_string();
                if !valid_bundle_id(&bundle_id)
                    || bundle_id == own_bundle_id
                    || !bundles.insert(bundle_id.clone())
                {
                    continue;
                }
                let name: Option<Retained<NSString>> = msg_send![&*application, localizedName];
                choices.push(SystemAudioApplication {
                    name: name
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| bundle_id.clone()),
                    bundle_id,
                });
            }
            choices.sort_by(|a, b| {
                a.name
                    .to_lowercase()
                    .cmp(&b.name.to_lowercase())
                    .then(a.bundle_id.cmp(&b.bundle_id))
            });
            Ok(choices)
        }))
    }));
    match result {
        Ok(Ok(value)) => value.map_err(str::to_string),
        _ => Err("macOS could not list running applications".into()),
    }
}

#[derive(Default)]
pub struct SystemAudioState {
    capture: Mutex<Option<CaptureProcess>>,
    // Separate lock: stop holds capture while joining readers, which register paths here.
    chunks: Arc<Mutex<BTreeSet<PathBuf>>>,
}

struct CaptureProcess {
    child: Child,
    readers: Vec<JoinHandle<()>>,
    directory: PathBuf,
}

impl CaptureProcess {
    fn stop(&mut self) -> Result<(), String> {
        #[cfg(unix)]
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        let mut exited = false;
        for _ in 0..30 {
            if self.child.try_wait().map_err(|e| e.to_string())?.is_some() {
                exited = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if !exited {
            self.child.kill().map_err(|e| e.to_string())?;
            self.child.wait().map_err(|e| e.to_string())?;
        }
        // EOF follows process exit. Return only after the final stdout path was registered/emitted.
        for reader in self.readers.drain(..) {
            reader
                .join()
                .map_err(|_| "capture output reader panicked".to_string())?;
        }
        if exited {
            Ok(())
        } else {
            Err("capture helper timed out; final audio may be incomplete".into())
        }
    }
}

impl Drop for CaptureProcess {
    fn drop(&mut self) {
        // Drop is the privacy fallback, not a save operation. Never leave an owned helper recording.
        let _ = self.child.kill();
        let _ = self.child.wait();
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        let _ = std::fs::remove_dir(&self.directory); // empty only; pending chunks remain readable
    }
}

impl Drop for SystemAudioState {
    fn drop(&mut self) {
        if let Ok(capture) = self.capture.get_mut() {
            drop(capture.take());
        }
        if let Ok(chunks) = self.chunks.lock() {
            for path in chunks.iter() {
                let _ = std::fs::remove_file(path);
                if let Some(dir) = path.parent() {
                    let _ = std::fs::remove_dir(dir);
                }
            }
        }
    }
}

fn valid_chunk_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("sysaudio_"))
        .and_then(|s| s.strip_suffix(".wav"))
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

fn validate_chunk(path: &Path, directory: &Path) -> Result<(), String> {
    if path.parent() != Some(directory) || !valid_chunk_name(path) {
        return Err("not an owned capture chunk".into());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("capture chunk is not a regular file".into());
    }
    Ok(())
}

/// The renderer suggests a name only. Native code creates the directory exclusively
/// with private permissions; pre-existing directories and non-temp parents are rejected.
fn create_capture_directory(requested: &str) -> Result<PathBuf, String> {
    let requested = Path::new(requested);
    let name = requested
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if !name.starts_with("permagent-audiocap-")
        || name.len() > 100
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("invalid capture directory name".into());
    }
    let parent = requested
        .parent()
        .ok_or("capture directory needs a parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    // Existing renderer versions use /tmp; macOS temp_dir() is normally a per-user folder.
    let temp = std::env::temp_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let conventional_temp = Path::new("/tmp").canonicalize().ok();
    if parent != temp && conventional_temp.as_ref() != Some(&parent) {
        return Err("capture directory must be in the system temporary directory".into());
    }
    let directory = parent.join(name);
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&directory)
        .map_err(|e| format!("cannot create private capture directory: {e}"))?;
    Ok(directory)
}

/// Locate the sidecar. Bundled next to the executable in a packaged app; in a
/// dev run it sits in the source tree where `build-audiocap.sh` compiled it.
fn sidecar_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled = dir.join("permagent-audiocap");
            if bundled.exists() {
                return Some(bundled);
            }
        }
    }
    if let Ok(res) = app.path().resource_dir() {
        let r = res.join("permagent-audiocap");
        if r.exists() {
            return Some(r);
        }
    }
    let dev =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("audiocap/permagent-audiocap");
    dev.exists().then_some(dev)
}

/// Start is serialized through spawn/registration; two invocations cannot own two helpers.
#[tauri::command]
pub async fn start_system_audio(
    app: AppHandle,
    out_dir: String,
    chunk_seconds: f64,
    include_app_bundle_id: Option<String>,
) -> Result<(), String> {
    validate_source_scope(include_app_bundle_id.as_deref(), &app.config().identifier)?;
    if !chunk_seconds.is_finite() || !(1.0..=300.0).contains(&chunk_seconds) {
        return Err("capture chunk duration must be between 1 and 300 seconds".into());
    }
    let state = app.state::<SystemAudioState>();
    let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
    if let Some(capture) = guard.as_mut() {
        if capture
            .child
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Err("system audio capture is already running".into());
        }
        // Reap a helper that failed after startup before permitting a new recording.
        drop(guard.take());
    }
    let bin = sidecar_path(&app).ok_or("system audio capture helper not found in this build")?;
    if include_app_bundle_id.is_some() {
        // Old helpers ignore unknown CLI flags. Never let a mixed-version app
        // silently turn an explicit application choice into all-system capture.
        let bytes =
            std::fs::read(&bin).map_err(|e| format!("cannot inspect capture helper: {e}"))?;
        if !bytes
            .windows(SCOPED_AUDIO_PROTOCOL.len())
            .any(|part| part == SCOPED_AUDIO_PROTOCOL)
        {
            return Err("this audio helper does not support application-scoped capture; rebuild or update the app".into());
        }
    }
    let directory = create_capture_directory(&out_dir)?;
    let mut command = Command::new(bin);
    command.args([
        "--out",
        &directory.to_string_lossy(),
        "--chunk-seconds",
        &chunk_seconds.to_string(),
        "--parent-pid",
        &std::process::id().to_string(),
        "--exclude-bundle-id",
        &app.config().identifier,
    ]);
    if let Some(bundle_id) = &include_app_bundle_id {
        command.args(["--include-bundle-id", bundle_id]);
    }
    let mut child = match command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            let _ = std::fs::remove_dir(&directory);
            return Err(format!("failed to start system audio capture: {e}"));
        }
    };
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let app_out = app.clone();
        let owned_chunks = state.chunks.clone();
        let owned_directory = directory.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let path = PathBuf::from(line.trim());
                if validate_chunk(&path, &owned_directory).is_err() {
                    continue;
                }
                if let Ok(mut chunks) = owned_chunks.lock() {
                    chunks.insert(path.clone());
                }
                let _ = app_out.emit("system_audio_chunk", path.to_string_lossy().as_ref());
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let app_err = app.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                if let Some(rest) = line.strip_prefix("ERROR ") {
                    let _ = app_err.emit("system_audio_error", serde_json::json!({
                        "kind": rest.split_whitespace().next().unwrap_or("unknown"), "detail": rest,
                    }));
                } else if line.starts_with("READY") {
                    let _ = app_err.emit("system_audio_ready", ());
                }
                eprintln!("[permagent-audiocap] {line}");
            }
        }));
    }
    *guard = Some(CaptureProcess {
        child,
        readers,
        directory,
    });
    Ok(())
}

/// Flush/join before resolving. The returned unread paths close the asynchronous
/// webview-event delivery race: clients deduplicate these against emitted paths.
#[tauri::command]
pub async fn stop_system_audio(app: AppHandle) -> Result<Vec<String>, String> {
    let state = app.state::<SystemAudioState>();
    let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
    let Some(mut capture) = guard.take() else {
        return Ok(Vec::new());
    };
    capture.stop()?;
    let chunks = state.chunks.lock().map_err(|e| e.to_string())?;
    Ok(chunks
        .iter()
        .filter(|p| p.parent() == Some(capture.directory.as_path()))
        .map(|p| p.to_string_lossy().into_owned())
        .collect())
}

#[tauri::command]
pub fn system_audio_available(app: AppHandle) -> bool {
    sidecar_path(&app).is_some()
}

/// Only paths emitted by an owned helper may be read, once. No-follow open
/// closes a symlink substitution race; private directories prevent other users
/// from replacing a chunk between validation and open.
#[tauri::command]
pub fn read_audio_chunk(app: AppHandle, path: String) -> Result<Vec<u8>, String> {
    let state = app.state::<SystemAudioState>();
    let p = PathBuf::from(path);
    let mut chunks = state.chunks.lock().map_err(|e| e.to_string())?;
    if !chunks.contains(&p) {
        return Err("not an owned unread capture chunk".into());
    }
    validate_chunk(&p, p.parent().ok_or("missing capture directory")?)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(&p).map_err(|e| e.to_string())?;
    // A maximum-length (300s) 16kHz PCM chunk is below 10 MB.
    if file.metadata().map_err(|e| e.to_string())?.len() > 10_000_000 {
        return Err("capture chunk exceeds maximum size".into());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    chunks.remove(&p);
    drop(chunks);
    // Do not remove an active helper's directory between chunks. Acquire locks
    // in separate scopes so stop's capture -> chunks order cannot deadlock.
    if let Some(dir) = p.parent() {
        let active = state.capture.lock().map_err(|e| e.to_string())?;
        if !active
            .as_ref()
            .is_some_and(|capture| capture.directory == dir)
        {
            let _ = std::fs::remove_dir(dir);
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_scope_is_explicit_and_cannot_include_parent() {
        let own = "ai.permagent.world-review-20261006";
        assert!(validate_source_scope(None, own).is_ok());
        assert!(validate_source_scope(Some("ai.permagent.record-practice-20261007"), own).is_ok());
        assert!(validate_source_scope(Some(own), own).is_err());
        for invalid in [
            "",
            "*",
            "com.apple.*",
            "com..apple",
            " com.apple.Safari",
            "com.apple.Safari\n",
        ] {
            assert!(
                validate_source_scope(Some(invalid), own).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn chunk_names_require_numeric_indices() {
        for name in ["sysaudio_0000.wav", "sysaudio_12000.wav"] {
            assert!(valid_chunk_name(Path::new(name)));
        }
        for name in ["sysaudio_.wav", "sysaudio_secret.wav", "sysaudio_1.wav.bak"] {
            assert!(!valid_chunk_name(Path::new(name)));
        }
    }

    #[cfg(unix)]
    #[test]
    fn stop_waits_for_final_stdout_before_returning() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let directory = create_capture_directory(
            std::env::temp_dir()
                .join(format!("permagent-audiocap-{}", uuid::Uuid::new_v4()))
                .to_str()
                .unwrap(),
        )
        .unwrap();
        // A deterministic fake helper, no ScreenCaptureKit or permissions.
        let mut child = Command::new("/bin/sh")
            .args([
                "-c",
                "trap 'printf tail; exit 0' TERM; printf 'ready\n'; while :; do :; done",
            ])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut ready = String::new();
        output.read_line(&mut ready).unwrap();
        assert_eq!(ready, "ready\n");
        let tail_seen = Arc::new(AtomicBool::new(false));
        let observed = tail_seen.clone();
        let reader = std::thread::spawn(move || {
            let mut tail = String::new();
            output.read_to_string(&mut tail).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
            observed.store(tail == "tail", Ordering::SeqCst);
        });
        let mut capture = CaptureProcess {
            child,
            readers: vec![reader],
            directory,
        };
        capture.stop().unwrap();
        assert!(tail_seen.load(Ordering::SeqCst));
        assert!(capture.child.try_wait().unwrap().is_some());
    }

    #[test]
    fn capture_directory_is_exclusive_and_rejects_unowned_paths() {
        let requested =
            std::env::temp_dir().join(format!("permagent-audiocap-{}", uuid::Uuid::new_v4()));
        let directory = create_capture_directory(requested.to_str().unwrap()).unwrap();
        assert!(create_capture_directory(requested.to_str().unwrap()).is_err());
        let chunk = directory.join("sysaudio_0000.wav");
        std::fs::write(&chunk, b"test").unwrap();
        assert!(validate_chunk(&chunk, &directory).is_ok());
        assert!(validate_chunk(&chunk, &directory.join("other")).is_err());
        #[cfg(unix)]
        {
            let link = directory.join("sysaudio_0001.wav");
            std::os::unix::fs::symlink(&chunk, &link).unwrap();
            assert!(validate_chunk(&link, &directory).is_err());
            std::fs::remove_file(link).unwrap();
        }
        std::fs::remove_file(chunk).unwrap();
        std::fs::remove_dir(directory).unwrap();
        assert!(create_capture_directory("/var/permagent-audiocap-invalid").is_err());
    }
}
