use std::sync::Mutex;
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandChild;

static DAEMON_CHILD: Mutex<Option<CommandChild>> = Mutex::new(None);

/// Spawn the permagentd sidecar as a child process.
/// In dev mode (no sidecar binary bundled), this will fail gracefully
/// and expect the daemon to be running externally.
pub fn start_daemon(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    if is_daemon_running() {
        eprintln!("[permagent-app] daemon already running on :3001, skipping spawn");
        return Ok(());
    }

    let sidecar = app.shell().sidecar("permagentd");
    match sidecar {
        Ok(cmd) => {
            let (mut rx, child) = cmd.args(["agent"]).spawn()?;

            *DAEMON_CHILD.lock().unwrap() = Some(child);

            tauri::async_runtime::spawn(async move {
                use tauri_plugin_shell::process::CommandEvent;
                while let Some(event) = rx.recv().await {
                    match event {
                        CommandEvent::Stdout(line) => {
                            eprintln!("[permagentd] {}", String::from_utf8_lossy(&line));
                        }
                        CommandEvent::Stderr(line) => {
                            eprintln!("[permagentd:err] {}", String::from_utf8_lossy(&line));
                        }
                        CommandEvent::Terminated(status) => {
                            eprintln!("[permagentd] exited with {:?}", status);
                            break;
                        }
                        _ => {}
                    }
                }
            });

            eprintln!("[permagent-app] daemon sidecar spawned");
        }
        Err(e) => {
            eprintln!(
                "[permagent-app] sidecar not available ({}), expecting external daemon",
                e
            );
        }
    }

    Ok(())
}

pub fn stop_daemon(_app: &tauri::AppHandle) {
    if let Some(child) = DAEMON_CHILD.lock().unwrap().take() {
        eprintln!("[permagent-app] stopping daemon sidecar");
        let _ = child.kill();
    }
}

fn is_daemon_running() -> bool {
    std::net::TcpStream::connect("127.0.0.1:3001").is_ok()
}
