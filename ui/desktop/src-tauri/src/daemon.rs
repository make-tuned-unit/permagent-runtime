use std::sync::Mutex;
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandChild;

static DAEMON_CHILD: Mutex<Option<CommandChild>> = Mutex::new(None);

/// Spawn the permagentd sidecar as a child process, then wait for it
/// to become ready on port 3001. If a daemon is already running
/// externally (e.g. via launchctl), we detect it and skip the spawn.
///
/// In dev mode the sidecar binary won't exist — we log a warning and
/// expect the developer to run the daemon separately.
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

            // Pipe daemon stdout/stderr so crashes are visible in Console.app
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

            eprintln!("[permagent-app] daemon sidecar spawned, waiting for readiness...");

            // Block until daemon is accepting connections (up to 10s)
            wait_for_daemon();
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

/// Kill the sidecar daemon when the app quits.
pub fn stop_daemon(_app: &tauri::AppHandle) {
    if let Some(child) = DAEMON_CHILD.lock().unwrap().take() {
        eprintln!("[permagent-app] stopping daemon sidecar");
        let _ = child.kill();
    }
}

/// Poll port 3001 until the daemon is ready, up to 10 seconds.
fn wait_for_daemon() {
    for i in 0..100 {
        if is_daemon_running() {
            eprintln!("[permagent-app] daemon ready after ~{}ms", i * 100);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    eprintln!("[permagent-app] WARNING: daemon did not become ready within 10s");
}

/// Quick check: is something listening on port 3001?
fn is_daemon_running() -> bool {
    std::net::TcpStream::connect_timeout(
        &"127.0.0.1:3001".parse().unwrap(),
        std::time::Duration::from_millis(50),
    )
    .is_ok()
}
