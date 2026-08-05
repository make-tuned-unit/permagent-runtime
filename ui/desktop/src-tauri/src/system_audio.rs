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

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Default)]
pub struct SystemAudioState(pub Mutex<Option<Child>>);

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

/// Begin capturing system audio. `chunk_seconds` controls transcription
/// granularity — the meeting recorder uses the same 45s cadence as its mic
/// chunks so the two streams interleave sensibly in the final transcript.
#[tauri::command]
pub async fn start_system_audio(
    app: AppHandle,
    out_dir: String,
    chunk_seconds: f64,
) -> Result<(), String> {
    let state = app.state::<SystemAudioState>();
    {
        let guard = state.0.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            // Already running: silently OK. A double-start from an impatient
            // second click must not spawn a second capture writing into the
            // same directory — two writers would interleave chunk indices and
            // silently corrupt the transcript ordering.
            return Ok(());
        }
    }

    let bin = sidecar_path(&app)
        .ok_or_else(|| "system audio capture helper not found in this build".to_string())?;

    let mut child = Command::new(&bin)
        .args([
            "--out",
            &out_dir,
            "--chunk-seconds",
            &chunk_seconds.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start system audio capture: {e}"))?;

    if let Some(out) = child.stdout.take() {
        let app_out = app.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let _ = app_out.emit("system_audio_chunk", line);
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let app_err = app.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                // Surface the sidecar's own diagnosis rather than a generic
                // failure: "permission" needs the user to grant Screen
                // Recording, everything else is ours to fix, and collapsing
                // them into one message is what makes this class of bug
                // undebuggable.
                if let Some(rest) = line.strip_prefix("ERROR ") {
                    let kind = rest.split_whitespace().next().unwrap_or("unknown");
                    let _ = app_err.emit(
                        "system_audio_error",
                        serde_json::json!({
                            "kind": kind,
                            "detail": rest,
                        }),
                    );
                } else if line.starts_with("READY") {
                    let _ = app_err.emit("system_audio_ready", ());
                }
                eprintln!("[permagent-audiocap] {line}");
            }
        });
    }

    *state.0.lock().map_err(|e| e.to_string())? = Some(child);
    Ok(())
}

/// Stop capture. SIGTERM (not kill) so the sidecar flushes the partial final
/// chunk — a meeting that ends mid-chunk must still transcribe its last words.
#[tauri::command]
pub async fn stop_system_audio(app: AppHandle) -> Result<(), String> {
    let state = app.state::<SystemAudioState>();
    let child = { state.0.lock().map_err(|e| e.to_string())?.take() };
    let Some(mut child) = child else {
        return Ok(());
    };

    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }

    // Give the flush a bounded moment, then insist. A stop button that hangs
    // on a wedged helper is worse than losing the tail of one chunk.
    for _ in 0..30 {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let _ = child.kill();
    Ok(())
}

/// Whether the capture helper exists in this build — lets the UI hide or
/// explain the toggle instead of offering a switch that cannot work.
#[tauri::command]
pub fn system_audio_available(app: AppHandle) -> bool {
    sidecar_path(&app).is_some()
}

/// Read a completed capture chunk so the renderer can POST it to the hub's
/// local Whisper. The sidecar writes to a temp directory the renderer has no
/// filesystem access to, and routing the bytes through here avoids granting
/// the webview broad file-read permissions for one narrow job. Deletes the
/// file after reading: a meeting is hours of audio and leaving it on disk
/// outlives the user's intent to record.
#[tauri::command]
pub fn read_audio_chunk(path: String) -> Result<Vec<u8>, String> {
    let p = std::path::Path::new(&path);
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    // Only ever read what the sidecar produced, whatever the caller claims.
    if !name.starts_with("sysaudio_") || !name.ends_with(".wav") {
        return Err("not a capture chunk".into());
    }
    let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(p);
    Ok(bytes)
}
