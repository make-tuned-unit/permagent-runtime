use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn portable_pty::Child + Send>,
}

pub type PtySessions = Arc<Mutex<HashMap<String, PtySession>>>;

pub fn new_sessions() -> PtySessions {
    Arc::new(Mutex::new(HashMap::new()))
}

#[derive(Clone, Serialize)]
struct PtyDataEvent {
    session_id: String,
    data: String,
}

#[derive(Clone, Serialize)]
struct PtyExitEvent {
    session_id: String,
    code: Option<u32>,
}

#[tauri::command]
pub fn spawn_pty_session(
    app: AppHandle,
    sessions: tauri::State<'_, PtySessions>,
    shell: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<String, String> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let pty_system = native_pty_system();

    let size = PtySize {
        rows: rows.unwrap_or(24),
        cols: cols.unwrap_or(80),
        pixel_width: 0,
        pixel_height: 0,
    };

    let pair = pty_system
        .openpty(size)
        .map_err(|e| format!("Failed to open PTY: {}", e))?;

    let shell_cmd = shell.unwrap_or_else(|| {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
    });

    let mut cmd = CommandBuilder::new(&shell_cmd);
    cmd.arg("-l"); // login shell

    if let Some(ref dir) = cwd {
        cmd.cwd(dir);
    } else if let Some(home) = dirs::home_dir() {
        cmd.cwd(home);
    }

    // Inherit environment
    for (key, val) in std::env::vars() {
        cmd.env(key, val);
    }
    cmd.env("TERM", "xterm-256color");

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn shell: {}", e))?;

    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("Failed to get PTY writer: {}", e))?;

    // Spawn reader thread that emits data events to the frontend
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("Failed to clone PTY reader: {}", e))?;

    let sid = session_id.clone();
    let app_handle = app.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // Convert bytes to string, replacing invalid UTF-8
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = app_handle.emit(
                        "pty_data",
                        PtyDataEvent {
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
            PtyExitEvent {
                session_id: sid.clone(),
                code: None,
            },
        );
    });

    let session = PtySession {
        master: pair.master,
        writer,
        _child: child,
    };

    sessions
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .insert(session_id.clone(), session);

    Ok(session_id)
}

#[tauri::command]
pub fn write_to_pty(
    sessions: tauri::State<'_, PtySessions>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    let mut map = sessions.lock().map_err(|e| format!("Lock error: {}", e))?;
    let session = map
        .get_mut(&session_id)
        .ok_or_else(|| format!("Session not found: {}", session_id))?;
    session
        .writer
        .write_all(data.as_bytes())
        .map_err(|e| format!("Write error: {}", e))?;
    session
        .writer
        .flush()
        .map_err(|e| format!("Flush error: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn resize_pty(
    sessions: tauri::State<'_, PtySessions>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let map = sessions.lock().map_err(|e| format!("Lock error: {}", e))?;
    let session = map
        .get(&session_id)
        .ok_or_else(|| format!("Session not found: {}", session_id))?;
    session
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Resize error: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn kill_pty(
    sessions: tauri::State<'_, PtySessions>,
    session_id: String,
) -> Result<(), String> {
    let mut map = sessions.lock().map_err(|e| format!("Lock error: {}", e))?;
    if let Some(mut session) = map.remove(&session_id) {
        let _ = session._child.kill();
    }
    Ok(())
}
