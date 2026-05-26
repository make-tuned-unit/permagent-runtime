/// Wait for the launchd-managed permagentd daemon to become ready
/// on port 3001. The daemon is NOT spawned as a Tauri sidecar —
/// it's owned by launchd via ~/Library/LaunchAgents/ai.permagent.daemon.plist
/// and bundled into the .app at Contents/MacOS/permagentd (referenced
/// by the launchd plist path).
///
/// Two spawners (Tauri sidecar + launchd) both binding port 3001
/// caused a crash loop where the loser was respawned every ~10s
/// by KeepAlive. Removed sidecar spawn so launchd is the single
/// source of truth for daemon lifecycle.
pub fn start_daemon(_app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    if is_daemon_running() {
        eprintln!("[permagent-app] daemon detected on :3001");
        return Ok(());
    }

    eprintln!("[permagent-app] daemon not yet ready, waiting...");
    wait_for_daemon();
    Ok(())
}

/// No-op — launchd owns daemon lifecycle.
pub fn stop_daemon(_app: &tauri::AppHandle) {
    // Daemon is managed by launchd. The app should not stop it.
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

/// Read the daemon Bearer token from ~/.permagent/secrets/daemon_token.json.
/// Returns the token string if available, or an error message.
#[tauri::command]
pub async fn get_daemon_token() -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let token_path = std::path::PathBuf::from(home)
        .join(".permagent")
        .join("secrets")
        .join("daemon_token.json");

    let content = std::fs::read_to_string(&token_path)
        .map_err(|e| format!("Failed to read daemon token: {}", e))?;

    let parsed: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse daemon token: {}", e))?;

    parsed
        .get("token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "daemon_token.json missing 'token' field".into())
}
