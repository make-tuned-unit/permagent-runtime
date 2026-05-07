use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn portable_pty::Child + Send>,
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

    let sid = session_id.clone();
    let app_handle = app.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = app_handle.emit(
                        "pty_data",
                        PtyDataPayload {
                            session_id: sid.clone(),
                            data,
                        },
                    );
                }
                Err(_) => break,
            }
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
    let mut writer = writer;
    if shell_path.contains("zsh") {
        let init = concat!(
            " autoload -Uz add-zsh-hook 2>/dev/null;",
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

#[tauri::command]
pub async fn write_to_pty(
    app: AppHandle,
    session_id: String,
    data: String,
) -> Result<(), String> {
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
