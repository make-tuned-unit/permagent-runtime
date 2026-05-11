use anyhow::{Context, Result};
use serde_json;
use std::path::PathBuf;
use std::process::Command;

const PLIST_LABEL: &str = "ai.permagent.daemon";
const DEFAULT_PORT: u16 = 3001;

fn plist_path() -> PathBuf {
    let home = dirs::home_dir().expect("could not determine home directory");
    home.join("Library/LaunchAgents/ai.permagent.daemon.plist")
}

fn logs_dir() -> PathBuf {
    let home = dirs::home_dir().expect("could not determine home directory");
    home.join(".permagent/logs")
}

fn config_path() -> PathBuf {
    let home = dirs::home_dir().expect("could not determine home directory");
    home.join(".permagent/config.yaml")
}

pub fn read_daemon_port() -> u16 {
    let path = config_path();
    if let Ok(contents) = std::fs::read_to_string(&path) {
        if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&contents) {
            if let Some(port) = yaml
                .get("daemon")
                .and_then(|d| d.get("port"))
                .and_then(|p| p.as_u64())
            {
                return port as u16;
            }
        }
    }
    DEFAULT_PORT
}

/// Returns the daemon WebSocket URL (ws://127.0.0.1:<port>).
pub fn daemon_ws_url() -> Result<String> {
    let port = read_daemon_port();
    Ok(format!("ws://127.0.0.1:{}", port))
}

/// Loads the daemon token from ~/.permagent/secrets/daemon_token.json.
pub fn load_daemon_token() -> Result<String> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    let path = home.join(".permagent/secrets/daemon_token.json");
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read daemon token from {}", path.display()))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&contents).context("failed to parse daemon_token.json")?;
    parsed
        .get("token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .context("daemon_token.json missing 'token' field")
}

fn find_permagentd_binary() -> Result<String> {
    // Try `which permagentd` first
    let output = Command::new("which")
        .arg("permagentd")
        .output()
        .context("failed to run `which permagentd`")?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(path);
        }
    }

    // Fallback: cargo build output
    let home = dirs::home_dir().expect("could not determine home directory");
    let cargo_path = home.join(".cargo/bin/permagentd");
    if cargo_path.exists() {
        return Ok(cargo_path.to_string_lossy().to_string());
    }

    anyhow::bail!(
        "permagentd binary not found. Build with `cargo install --path crates/goose-server` or ensure it is on PATH."
    )
}

fn generate_plist(binary_path: &str, port: u16) -> String {
    let home = dirs::home_dir().expect("could not determine home directory");
    let home_str = home.to_string_lossy();
    let cargo_bin = home.join(".cargo/bin").to_string_lossy().to_string();

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{PLIST_LABEL}</string>

    <key>ProgramArguments</key>
    <array>
        <string>{binary_path}</string>
        <string>agent</string>
        <string>--host</string>
        <string>127.0.0.1</string>
        <string>--port</string>
        <string>{port}</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>

    <key>StandardOutPath</key>
    <string>{home_str}/.permagent/logs/daemon.log</string>

    <key>StandardErrorPath</key>
    <string>{home_str}/.permagent/logs/daemon.err</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>{home_str}</string>
        <key>PATH</key>
        <string>{cargo_bin}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
        <key>PERMAGENT_CONFIG</key>
        <string>{home_str}/.permagent/config.yaml</string>
        <key>PERMAGENT_SPECTRAL_DB</key>
        <string>{home_str}/.permagent/spectral/permagent.db</string>
    </dict>

    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
"#
    )
}

pub fn handle_start() -> Result<()> {
    let plist = plist_path();
    let port = read_daemon_port();

    // Ensure logs directory exists
    let logs = logs_dir();
    std::fs::create_dir_all(&logs).context("failed to create logs directory")?;

    // Generate plist if it doesn't exist
    if !plist.exists() {
        let binary_path = find_permagentd_binary()?;
        let content = generate_plist(&binary_path, port);

        // Ensure LaunchAgents directory exists
        if let Some(parent) = plist.parent() {
            std::fs::create_dir_all(parent).context("failed to create LaunchAgents directory")?;
        }

        std::fs::write(&plist, &content)
            .with_context(|| format!("failed to write plist to {}", plist.display()))?;
        println!("Generated plist at {}", plist.display());
    }

    // Load via launchctl
    let status = Command::new("launchctl")
        .args(["load", &plist.to_string_lossy()])
        .status()
        .context("failed to run launchctl load")?;

    if status.success() {
        println!("Permagent daemon started (port {port})");
    } else {
        eprintln!("launchctl load exited with status {status}");
        eprintln!("The daemon may already be loaded. Try: permagent restart");
    }

    Ok(())
}

pub fn handle_stop() -> Result<()> {
    let plist = plist_path();

    if !plist.exists() {
        println!(
            "Daemon plist not found at {}. Nothing to stop.",
            plist.display()
        );
        return Ok(());
    }

    let status = Command::new("launchctl")
        .args(["unload", &plist.to_string_lossy()])
        .status()
        .context("failed to run launchctl unload")?;

    if status.success() {
        println!("Permagent daemon stopped.");
    } else {
        eprintln!("launchctl unload exited with status {status}. Daemon may not be running.");
    }

    Ok(())
}

pub fn handle_restart() -> Result<()> {
    println!("Stopping daemon...");
    let _ = handle_stop();
    println!("Starting daemon...");
    handle_start()
}

pub fn handle_status() -> Result<()> {
    let port = read_daemon_port();

    // Check launchctl list
    let output = Command::new("launchctl")
        .args(["list"])
        .output()
        .context("failed to run launchctl list")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut found = false;

    for line in stdout.lines() {
        if line.contains(PLIST_LABEL) {
            found = true;
            let parts: Vec<&str> = line.split_whitespace().collect();
            let pid = parts.first().unwrap_or(&"-");
            let exit_code = parts.get(1).unwrap_or(&"-");

            if *pid != "-" {
                println!("Status:    running");
                println!("PID:       {pid}");
                println!("Port:      {port}");
                // Try to get uptime from ps
                if let Ok(ps_out) = Command::new("ps")
                    .args(["-o", "etime=", "-p", pid])
                    .output()
                {
                    let etime = String::from_utf8_lossy(&ps_out.stdout).trim().to_string();
                    if !etime.is_empty() {
                        println!("Uptime:    {etime}");
                    }
                }
            } else {
                println!("Status:    not running (exit code: {exit_code})");
                println!("Port:      {port} (configured)");
            }
            break;
        }
    }

    if !found {
        println!("Status:    not loaded");
        println!("Port:      {port} (configured)");
        println!("\nRun `permagent start` to start the daemon.");
        return Ok(());
    }

    // Check if port is listening
    let listening = Command::new("lsof")
        .args(["-i", &format!(":{port}"), "-sTCP:LISTEN"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    println!(
        "Listening:  {}",
        if listening {
            format!("yes (port {port})")
        } else {
            format!("no (port {port} not open)")
        }
    );

    Ok(())
}

pub fn handle_logs(err: bool) -> Result<()> {
    let logs = logs_dir();
    let file = if err {
        logs.join("daemon.err")
    } else {
        logs.join("daemon.log")
    };

    if !file.exists() {
        println!("Log file not found: {}", file.display());
        println!("Start the daemon first with `permagent start`.");
        return Ok(());
    }

    println!("Tailing {} (Ctrl+C to stop)...", file.display());

    let status = Command::new("tail")
        .args(["-f", &file.to_string_lossy()])
        .status()
        .context("failed to tail log file")?;

    if !status.success() {
        eprintln!("tail exited with status {status}");
    }

    Ok(())
}

pub fn handle_open() -> Result<()> {
    // Try to launch the Tauri desktop app first
    let app_path = "/Applications/Permagent.app";
    if std::path::Path::new(app_path).exists() {
        println!("Launching Permagent desktop app");
        Command::new("open")
            .arg("-a")
            .arg(app_path)
            .status()
            .context("failed to launch Permagent desktop app")?;
        return Ok(());
    }

    // Fall back to browser
    let port = read_daemon_port();
    let url = format!("http://localhost:{port}/ui/");

    println!("Opening {url}");

    Command::new("open")
        .arg(&url)
        .status()
        .context("failed to open browser")?;

    Ok(())
}
