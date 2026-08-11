use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{mpsc, Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

// ── Supervised-session tee (S2, #428) ───────────────────────────────────────
//
// A SUPERVISED Claude Code session (epic #399) runs its stream-json protocol
// inside this visible PTY. The daemon's gate parser + session registry live in
// the daemon process, so the PTY reader tees every raw output chunk of a
// supervised session to the daemon's ingest route. Plain terminals never tee —
// the frontend passes `supervised_session_id` only for launches tagged by the
// daemon's `project_launch` event.
//
// Ordering matters (the parser reassembles NDJSON lines across chunks), so a
// single forwarder thread drains an mpsc channel sequentially — never parallel
// fire-and-forget POSTs. The daemon address/port mirror daemon.rs.
const DAEMON_SUPERVISED_OUTPUT_URL: &str = "http://127.0.0.1:3001/terminal/supervised/output";
/// Coalesce bursts up to this many bytes per POST so heavy output doesn't
/// become a POST-per-4KiB-read storm.
const TEE_COALESCE_CAP_BYTES: usize = 64 * 1024;
/// Give up teeing after this many consecutive delivery failures (daemon down);
/// gates can't be observed anyway and the session stays visible in the tab.
const TEE_MAX_CONSECUTIVE_FAILURES: u32 = 20;

enum TeeFrame {
    Data(String),
    Eof,
}

/// Drain-and-coalesce: starting from `first`, greedily append already-queued
/// Data frames (up to `cap` bytes) and report whether an Eof was consumed.
/// Pure so the batching contract is unit-testable.
fn coalesce_frames(first: TeeFrame, rx: &mpsc::Receiver<TeeFrame>, cap: usize) -> (String, bool) {
    let mut data = String::new();
    let mut eof = false;
    match first {
        TeeFrame::Data(d) => data.push_str(&d),
        TeeFrame::Eof => eof = true,
    }
    while !eof && data.len() < cap {
        match rx.try_recv() {
            Ok(TeeFrame::Data(d)) => data.push_str(&d),
            Ok(TeeFrame::Eof) => eof = true,
            Err(_) => break,
        }
    }
    (data, eof)
}

/// Spawn the forwarder thread for one supervised session and return the
/// sender the PTY reader feeds. Failure posture: log-and-continue on
/// transient errors, stop for good on 404 (the daemon no longer knows the
/// session — e.g. it restarted) or after sustained failure; the reader's
/// sends then drop silently (`let _`), never blocking the terminal.
fn spawn_supervised_tee(
    supervised_session_id: String,
    pty_session_id: String,
) -> mpsc::Sender<TeeFrame> {
    let (tx, rx) = mpsc::channel::<TeeFrame>();
    std::thread::spawn(move || {
        let token = match crate::daemon::read_daemon_token() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[permagent-app] supervised tee: no daemon token, not teeing: {e}");
                return;
            }
        };
        let client = reqwest::Client::new();
        let mut consecutive_failures: u32 = 0;
        loop {
            let first = match rx.recv() {
                Ok(f) => f,
                // Reader thread gone without an explicit Eof (shouldn't
                // happen — it always sends Eof) — treat as end of stream.
                Err(_) => TeeFrame::Eof,
            };
            let (data, eof) = coalesce_frames(first, &rx, TEE_COALESCE_CAP_BYTES);
            let body = serde_json::json!({
                "supervised_session_id": supervised_session_id,
                "pty_session_id": pty_session_id,
                "data": data,
                "eof": eof,
            });
            let send = client
                .post(DAEMON_SUPERVISED_OUTPUT_URL)
                .bearer_auth(&token)
                .json(&body)
                .send();
            match tauri::async_runtime::block_on(send) {
                Ok(resp) if resp.status().as_u16() == 404 => {
                    eprintln!(
                        "[permagent-app] supervised tee: daemon does not know session {supervised_session_id} — stopping tee"
                    );
                    return;
                }
                Ok(resp) if !resp.status().is_success() => {
                    consecutive_failures += 1;
                    eprintln!(
                        "[permagent-app] supervised tee: ingest returned {} for {supervised_session_id}",
                        resp.status()
                    );
                }
                Ok(_) => consecutive_failures = 0,
                Err(e) => {
                    consecutive_failures += 1;
                    if consecutive_failures == 1 {
                        eprintln!(
                            "[permagent-app] supervised tee: delivery failed for {supervised_session_id}: {e}"
                        );
                    }
                }
            }
            if consecutive_failures >= TEE_MAX_CONSECUTIVE_FAILURES {
                eprintln!(
                    "[permagent-app] supervised tee: {TEE_MAX_CONSECUTIVE_FAILURES} consecutive failures — stopping tee for {supervised_session_id}"
                );
                return;
            }
            if eof {
                return;
            }
        }
    });
    tx
}

struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn portable_pty::Child + Send>,
    output: Arc<Mutex<String>>,
    /// Stream position matching `PtyDataPayload::seq`, so a replay can say
    /// exactly which emitted chunks it already contains.
    produced: Arc<std::sync::atomic::AtomicU64>,
}

pub struct PtySessions(Mutex<HashMap<String, PtySession>>);

impl PtySessions {
    pub fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

#[derive(Clone, Serialize)]
struct PtyDataPayload {
    session_id: String,
    data: String,
    /// Total bytes this session has produced, INCLUDING this chunk.
    ///
    /// The replay buffer and this event stream carry the same bytes, so a
    /// client that subscribes and then fetches the replay would write the
    /// overlap twice — splicing stale output into whatever the TUI is drawing
    /// now. `seq` lets the client discard exactly the chunks the replay
    /// already contains. See `get_pty_output`.
    seq: u64,
}

#[derive(Clone, Serialize)]
pub struct SpawnResult {
    session_id: String,
    cwd: String,
}

#[derive(Clone, Serialize)]
struct PtyExitPayload {
    session_id: String,
    code: Option<u32>,
}

#[tauri::command]
pub async fn spawn_pty_session(
    app: AppHandle,
    shell: Option<String>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    supervised_session_id: Option<String>,
) -> Result<SpawnResult, String> {
    let session_id = format!("pty-{}", uuid::Uuid::new_v4());

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Failed to open PTY: {e}"))?;

    let shell_path = shell.unwrap_or_else(|| {
        let env_shell = std::env::var("SHELL").unwrap_or_default();
        // Prefer zsh on macOS — bash is deprecated and lacks OSC 7 CWD reporting.
        if env_shell.is_empty() || env_shell == "/bin/bash" {
            if std::path::Path::new("/bin/zsh").exists() {
                "/bin/zsh".to_string()
            } else {
                env_shell
            }
        } else {
            env_shell
        }
    });

    // Resolve the working directory: use provided cwd, or fall back to HOME.
    let resolved_cwd = cwd
        .clone()
        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));

    let mut cmd = CommandBuilder::new(&shell_path);
    cmd.arg("-l"); // login shell
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    // Remove env vars that prevent tools from running inside Permagent's terminal.
    // CLAUDECODE is set by Claude Code sessions and blocks nested `claude` invocations.
    cmd.env_remove("CLAUDECODE");
    cmd.env_remove("CLAUDE_CODE_SESSION");
    if let Some(dir) = cwd {
        cmd.cwd(dir);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn shell: {e}"))?;

    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("Failed to get PTY writer: {e}"))?;

    // Spawn reader thread to forward PTY output
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("Failed to get PTY reader: {e}"))?;

    // S2 (#428): supervised sessions tee raw output to the daemon's gate
    // parser. `None` for plain terminals — zero overhead on the common path.
    let tee_tx = supervised_session_id
        .as_ref()
        .map(|sup| spawn_supervised_tee(sup.clone(), session_id.clone()));

    let sid = session_id.clone();
    let app_handle = app.clone();
    let output = Arc::new(Mutex::new(String::new()));
    let reader_output = output.clone();
    // Monotonic byte count, never reset by replay-buffer truncation — it is a
    // stream POSITION, not a length.
    let produced = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let reader_produced = produced.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    let seq = reader_produced
                        .fetch_add(data.len() as u64, std::sync::atomic::Ordering::SeqCst)
                        + data.len() as u64;
                    if let Ok(mut replay) = reader_output.lock() {
                        replay.push_str(&data);
                        const MAX_REPLAY_BYTES: usize = 2 * 1024 * 1024;
                        if replay.len() > MAX_REPLAY_BYTES {
                            let mut cut = replay.len() - MAX_REPLAY_BYTES;
                            while !replay.is_char_boundary(cut) {
                                cut += 1;
                            }
                            replay.drain(..cut);
                        }
                    }
                    if let Some(tx) = &tee_tx {
                        // Never blocks (unbounded); a dead forwarder is fine.
                        let _ = tx.send(TeeFrame::Data(data.clone()));
                    }
                    let _ = app_handle.emit(
                        "pty_data",
                        PtyDataPayload {
                            session_id: sid.clone(),
                            data,
                            seq,
                        },
                    );
                }
                Err(_) => break,
            }
        }
        if let Some(tx) = &tee_tx {
            // Tell the daemon the PTY closed: a supervised session that dies
            // without a `type:"result"` is failed through the completion seam
            // instead of parking at its goal timeout.
            let _ = tx.send(TeeFrame::Eof);
        }
        let _ = app_handle.emit(
            "pty_exit",
            PtyExitPayload {
                session_id: sid,
                code: None,
            },
        );
    });

    // Inject OSC 7 precmd hook so zsh reports CWD changes to the terminal.
    // macOS zsh only does this for Apple_Terminal — we need our own hook.
    // Leading space suppresses zsh history recording (HIST_IGNORE_SPACE).
    //
    // The OSC 133;D hook (semantic-prompt "command finished" mark, carrying
    // `$?`) MUST be registered BEFORE the OSC 7 hook: `$?` inside the first
    // precmd function is the last user command's exit status; later hooks see
    // the previous hook's status instead. Terminal.tsx pairs the mark with the
    // command it sniffed at Enter to emit terminal_command_completed — the
    // initiative layer's (#360) input signal. OSC 7 emission is unchanged.
    let mut writer = writer;
    if shell_path.contains("zsh") {
        let init = concat!(
            " autoload -Uz add-zsh-hook 2>/dev/null;",
            " __permagent_osc133d() { printf '\\e]133;D;%s\\a' \"$?\" };",
            " add-zsh-hook precmd __permagent_osc133d;",
            " __permagent_osc7() { printf '\\e]7;file://%s%s\\a' \"${HOST}\" \"${PWD}\" };",
            " add-zsh-hook precmd __permagent_osc7;",
            " clear\n",
        );
        let _ = writer.write_all(init.as_bytes());
        let _ = writer.flush();
    }

    let session = PtySession {
        master: pair.master,
        writer,
        _child: child,
        output,
        produced,
    };

    app.state::<PtySessions>()
        .0
        .lock()
        .unwrap()
        .insert(session_id.clone(), session);

    Ok(SpawnResult {
        session_id,
        cwd: resolved_cwd,
    })
}

/// The bounded scrollback, plus the stream position it covers.
///
/// A reattaching terminal subscribes to `pty_data` BEFORE calling this (so no
/// bytes are missed during the round trip) and then drops every queued chunk
/// whose `seq` is <= `seq` here. Without that the client wrote the overlap
/// twice: live output first, then the replay containing the same bytes on top
/// of it, splicing stale text into the line the TUI was drawing — reported
/// 2026-08-04 as harness text stuck in Claude Code's input box.
#[derive(Clone, Serialize)]
pub struct PtyReplay {
    data: String,
    seq: u64,
}

#[tauri::command]
pub async fn get_pty_output(app: AppHandle, session_id: String) -> Result<PtyReplay, String> {
    let sessions = app.state::<PtySessions>();
    let map = sessions.0.lock().unwrap();
    let session = map
        .get(&session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    // Read the position FIRST. If a chunk lands between these two reads the
    // replay contains bytes beyond `seq`, so the client re-writes a little —
    // harmless. The reverse order could drop bytes entirely.
    let seq = session.produced.load(std::sync::atomic::Ordering::SeqCst);
    let data = session
        .output
        .lock()
        .map(|s| s.clone())
        .map_err(|_| "Output buffer unavailable".to_string())?;
    Ok(PtyReplay { data, seq })
}

#[tauri::command]
pub async fn write_to_pty(app: AppHandle, session_id: String, data: String) -> Result<(), String> {
    let sessions = app.state::<PtySessions>();
    let mut map = sessions.0.lock().unwrap();
    let session = map
        .get_mut(&session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    session
        .writer
        .write_all(data.as_bytes())
        .map_err(|e| format!("Write failed: {e}"))?;
    session
        .writer
        .flush()
        .map_err(|e| format!("Flush failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn resize_pty(
    app: AppHandle,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let sessions = app.state::<PtySessions>();
    let map = sessions.0.lock().unwrap();
    let session = map
        .get(&session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    session
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Resize failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn kill_pty(app: AppHandle, session_id: String) -> Result<(), String> {
    let sessions = app.state::<PtySessions>();
    let mut map = sessions.0.lock().unwrap();
    if let Some(mut session) = map.remove(&session_id) {
        let _ = session._child.kill();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesce_merges_queued_data_in_order() {
        let (tx, rx) = mpsc::channel();
        tx.send(TeeFrame::Data("b".to_string())).unwrap();
        tx.send(TeeFrame::Data("c".to_string())).unwrap();
        let (data, eof) = coalesce_frames(TeeFrame::Data("a".to_string()), &rx, 1024);
        assert_eq!(data, "abc");
        assert!(!eof);
    }

    #[test]
    fn coalesce_stops_at_eof_and_reports_it() {
        let (tx, rx) = mpsc::channel();
        tx.send(TeeFrame::Eof).unwrap();
        tx.send(TeeFrame::Data("late".to_string())).unwrap();
        let (data, eof) = coalesce_frames(TeeFrame::Data("a".to_string()), &rx, 1024);
        assert_eq!(data, "a");
        assert!(eof, "eof frame must terminate the batch");
        // The late frame stays queued (it is dead data after eof — the
        // forwarder exits after posting the eof batch).
    }

    #[test]
    fn coalesce_respects_byte_cap() {
        let (tx, rx) = mpsc::channel();
        tx.send(TeeFrame::Data("y".repeat(10))).unwrap();
        tx.send(TeeFrame::Data("z".repeat(10))).unwrap();
        let (data, eof) = coalesce_frames(TeeFrame::Data("x".repeat(10)), &rx, 15);
        // First append crosses the cap, second stays queued for the next POST.
        assert_eq!(data.len(), 20);
        assert!(!eof);
        let (rest, _) = coalesce_frames(rx.recv().unwrap(), &rx, 15);
        assert_eq!(rest, "z".repeat(10));
    }

    #[test]
    fn eof_first_frame_posts_empty_final_batch() {
        let (_tx, rx) = mpsc::channel::<TeeFrame>();
        let (data, eof) = coalesce_frames(TeeFrame::Eof, &rx, 1024);
        assert_eq!(data, "");
        assert!(eof);
    }
}
