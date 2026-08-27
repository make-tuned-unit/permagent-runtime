//! Polybot — the user's prediction-market bot, as a control plane.
//!
//! Polybot (`~/…/polybot`) already sizes, places, and pauses its own
//! Polymarket orders. This module does not import CLOB keys and never talks
//! to Polymarket. It locates the checkout, reads `logs/bankroll.json`, and
//! drives the process the bot was already built to run as: launchd
//! `com.polybot.bot`, the `PAUSED` kill switch, and a scan-trigger file the
//! main loop already watches.
//!
//! A missing vault, a May bankroll, or a dead pid is a first-class answer,
//! not an empty zeroed card.

use chrono::Timelike;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Config key overriding where the Polybot checkout lives.
pub const POLYBOT_ROOT_KEY: &str = "polybot_root";
/// Optional path to the env file Polybot should load. Same key the bot reads
/// as `POLYBOT_VAULT`.
pub const POLYBOT_VAULT_KEY: &str = "polybot_vault";
/// Off until the user turns Polybot on from the Finance tab, after the risk
/// disclaimer. The card and Start path stay dark without this.
pub const POLYBOT_ENABLED_KEY: &str = "polybot_enabled";

/// True only when the user has opted in. Missing key is off.
pub fn is_enabled() -> bool {
    crate::config::Config::global()
        .get_param::<bool>(POLYBOT_ENABLED_KEY)
        .unwrap_or(false)
}

pub const LAUNCH_LABEL: &str = "com.polybot.bot";
const FULL_SCAN_FILE: &str = ".full_scan_trigger";
const CHECKOUT_NAMES: &[&str] = &["polybot", "Polybot"];

/// After this long without a bankroll write, the numbers are labelled stale.
const STALE_AFTER: Duration = Duration::from_secs(48 * 3600);

/// Keys the Finance / Settings UI writes to the keychain. Presence only —
/// values never leave `Config`.
pub const POLYMARKET_SECRET_KEYS: &[&str] = &[
    "POLYMARKET_API_KEY",
    "POLYMARKET_API_SECRET",
    "POLYMARKET_API_PASSPHRASE",
    "POLYMARKET_WALLET_PRIVATE_KEY",
    "POLYMARKET_FUNDER_ADDRESS",
];

const REQUIRED_KEYS: &[&str] = &[
    "POLYMARKET_API_KEY",
    "POLYMARKET_API_SECRET",
    "POLYMARKET_API_PASSPHRASE",
    "POLYMARKET_FUNDER_ADDRESS",
];

const WALLET_KEYS: &[&str] = &["POLYMARKET_WALLET_PRIVATE_KEY", "POLYMARKET_PRIVATE_KEY"];

/// Locate the Polybot checkout. `polybot_root` wins; otherwise the shared
/// `dev_roots` resolver, so a move between `~/dev` and `~/Documents/dev`
/// does not silently empty the card. APFS is case-insensitive; we still
/// try both spellings so a case-sensitive disk finds `polybot`.
pub fn polybot_root() -> Option<PathBuf> {
    if let Ok(configured) = crate::config::Config::global().get_param::<String>(POLYBOT_ROOT_KEY) {
        let p = PathBuf::from(shellexpand::tilde(&configured).into_owned());
        if looks_like_polybot(&p) {
            return Some(p);
        }
    }
    crate::config::dev_roots::dev_roots()
        .into_iter()
        .find_map(|root| first_checkout_under(&root))
}

fn first_checkout_under(dev_root: &Path) -> Option<PathBuf> {
    CHECKOUT_NAMES
        .iter()
        .map(|n| dev_root.join(n))
        .find(|p| looks_like_polybot(p))
}

fn looks_like_polybot(p: &Path) -> bool {
    p.is_dir() && p.join("main.py").is_file() && p.join("config.py").is_file()
}

/// What the Finance tab and the Financier see for Polybot.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolybotStatus {
    pub found: bool,
    pub root: Option<String>,
    pub running: bool,
    pub pid: Option<u32>,
    pub paused: bool,
    pub credentials_ready: bool,
    /// Where presence was proven: `keychain`, `process environment`, or a path.
    /// Never contains secrets.
    pub credentials_path: Option<String>,
    pub scan_requested: bool,
    pub quiet_hours: bool,
    pub current_balance: Option<f64>,
    pub realized_pnl: Option<f64>,
    pub open_exposure: Option<f64>,
    pub trade_count: Option<u64>,
    pub last_updated: Option<String>,
    /// Best clock we have for the numbers (bankroll `last_updated`, else
    /// `last_wallet_sync`). The UI must show this next to the balance so a
    /// May snapshot cannot be read as this morning's wallet.
    pub as_of: Option<String>,
    /// Whole days since [`as_of`]. None when we have no parseable clock.
    pub stale_days: Option<u64>,
    pub stale: bool,
    /// Why it is missing or unreadable, when it is.
    pub detail: Option<String>,
}

/// Read Polybot's on-disk state. Never calls Polymarket.
pub fn status() -> PolybotStatus {
    let Some(root) = polybot_root() else {
        return PolybotStatus {
            detail: Some(
                "no Polybot checkout found — set polybot_root or keep it under a known code directory"
                    .into(),
            ),
            ..Default::default()
        };
    };
    status_from_root(&root)
}

fn status_from_root(root: &Path) -> PolybotStatus {
    let creds = credentials_at(root);
    let pid = live_pid(root);
    let mut out = PolybotStatus {
        found: looks_like_polybot(root) || root.is_dir(),
        root: Some(root.display().to_string()),
        running: pid.is_some(),
        pid,
        paused: root.join("PAUSED").is_file(),
        credentials_ready: creds.ready,
        credentials_path: creds.path.clone(),
        scan_requested: root.join(FULL_SCAN_FILE).is_file(),
        quiet_hours: is_quiet_hours(),
        ..Default::default()
    };
    let bankroll_path = root.join("logs/bankroll.json");
    let Ok(raw) = std::fs::read_to_string(&bankroll_path) else {
        out.detail = Some(compose_detail(
            &out,
            &creds,
            format!(
                "no bankroll at {} — Polybot has not written status yet",
                bankroll_path.display()
            ),
        ));
        return out;
    };
    let v: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            out.detail = Some(compose_detail(
                &out,
                &creds,
                format!("bankroll.json is unreadable: {e}"),
            ));
            return out;
        }
    };
    out.current_balance = num_field(&v, "current_balance")
        .or_else(|| num_field(&v, "cash_balance"))
        .or_else(|| num_field(&v, "last_wallet_balance"));
    out.realized_pnl = num_field(&v, "realized_pnl");
    out.open_exposure = num_field(&v, "open_exposure");
    out.trade_count = v
        .get("trade_count")
        .and_then(|n| n.as_u64().or_else(|| n.as_f64().map(|f| f as u64)));
    out.last_updated = string_field(&v, "last_updated");
    let as_of = out
        .last_updated
        .clone()
        .or_else(|| string_field(&v, "last_wallet_sync"));
    out.as_of = as_of.clone();
    let age = as_of.as_deref().and_then(signed_age);
    out.stale_days = age.and_then(|d| u64::try_from(d.num_days()).ok());
    out.stale = match age {
        Some(d) if d.num_seconds() < 0 => false,
        Some(d) => d.to_std().is_ok_and(|std| std > STALE_AFTER),
        None => true,
    };
    let mut reason = if out.stale {
        match out.stale_days {
            Some(0) => "bankroll has not been written in 48 hours".into(),
            Some(days) => format!(
                "last live write {} — {days} days ago. Start Polybot to refresh; this tab does not call Polymarket.",
                as_of.as_deref().unwrap_or("unknown")
            ),
            None => "bankroll has no parseable timestamp — treating the numbers as stale".into(),
        }
    } else {
        String::new()
    };
    if !out.running && !out.paused {
        if !reason.is_empty() {
            reason.push(' ');
        }
        reason.push_str("Process is down.");
    }
    if out.quiet_hours && out.running {
        if !reason.is_empty() {
            reason.push(' ');
        }
        reason.push_str("Quiet hours (00:00–10:00 UTC) — full scans are skipped until then.");
    }
    out.detail = Some(compose_detail(&out, &creds, reason)).filter(|s| !s.is_empty());
    out
}

fn compose_detail(status: &PolybotStatus, creds: &CredentialProbe, reason: String) -> String {
    let mut parts = Vec::new();
    if !reason.is_empty() {
        parts.push(reason);
    }
    if status.paused {
        parts.push("kill switch PAUSED is set — resume to trade again".into());
    }
    if !creds.ready {
        parts.push(creds.detail.clone());
    }
    parts.join(" ")
}

struct CredentialProbe {
    ready: bool,
    path: Option<String>,
    detail: String,
}

fn credentials_at(root: &Path) -> CredentialProbe {
    let from_keychain = keys_from_keychain();
    if keys_ready(&from_keychain) {
        return CredentialProbe {
            ready: true,
            path: Some("keychain".into()),
            detail: String::new(),
        };
    }
    let present = {
        let mut p = present_env_keys();
        p.extend(from_keychain.iter().cloned());
        p
    };
    if keys_ready(&present) {
        return CredentialProbe {
            ready: true,
            path: Some("process environment".into()),
            detail: String::new(),
        };
    }
    for path in vault_candidates(root) {
        if !path.is_file() {
            continue;
        }
        let from_file = keys_in_env_file(&path);
        let mut merged = present.clone();
        merged.extend(from_file);
        if keys_ready(&merged) {
            return CredentialProbe {
                ready: true,
                path: Some(path.display().to_string()),
                detail: String::new(),
            };
        }
        return CredentialProbe {
            ready: false,
            path: Some(path.display().to_string()),
            detail: format!(
                "credentials at {} are missing {}. Prefer Settings → Search & tools (keychain).",
                path.display(),
                missing_keys(&merged).join(", ")
            ),
        };
    }
    CredentialProbe {
        ready: false,
        path: None,
        detail: format!(
            "Polymarket keys are not in the keychain. Add them on the Finance tab or in \
             Settings → Search & tools. Missing {}.",
            missing_keys(&present).join(", ")
        ),
    }
}

fn vault_candidates(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(configured) = crate::config::Config::global().get_param::<String>(POLYBOT_VAULT_KEY) {
        let p = PathBuf::from(shellexpand::tilde(&configured).into_owned());
        if !p.as_os_str().is_empty() {
            out.push(p);
        }
    }
    if let Ok(env) = std::env::var("POLYBOT_VAULT") {
        let p = PathBuf::from(shellexpand::tilde(&env).into_owned());
        if !p.as_os_str().is_empty() {
            out.push(p);
        }
    }
    if let Some(home) = dirs::home_dir() {
        out.push(root.join("credentials.env"));
        out.push(home.join(".permagent/polymarket.env"));
        out.push(home.join(".openclaw/vault/config/polymarket/credentials.env"));
    } else {
        out.push(root.join("credentials.env"));
    }
    let mut seen = HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

fn all_secret_names() -> impl Iterator<Item = &'static str> {
    REQUIRED_KEYS
        .iter()
        .copied()
        .chain(WALLET_KEYS.iter().copied())
}

fn present_env_keys() -> HashSet<String> {
    let mut keys = HashSet::new();
    for key in all_secret_names() {
        if std::env::var(key)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .is_some()
        {
            keys.insert(key.to_string());
        }
    }
    keys
}

fn keys_from_keychain() -> HashSet<String> {
    let cfg = crate::config::Config::global();
    let mut keys = HashSet::new();
    for key in all_secret_names() {
        if cfg
            .get_secret::<String>(key)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .is_some()
        {
            keys.insert(key.to_string());
        }
    }
    keys
}

fn secret_value(key: &str) -> Option<String> {
    if let Ok(v) = std::env::var(key) {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    crate::config::Config::global()
        .get_secret::<String>(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn keys_in_env_file(path: &Path) -> HashSet<String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashSet::new();
    };
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || !line.contains('=') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            let k = k.trim();
            if v.trim().is_empty() {
                return None;
            }
            Some(k.to_string())
        })
        .collect()
}

fn keys_ready(present: &HashSet<String>) -> bool {
    REQUIRED_KEYS.iter().all(|k| present.contains(*k))
        && (present.contains("POLYMARKET_WALLET_PRIVATE_KEY")
            || present.contains("POLYMARKET_PRIVATE_KEY"))
}

fn missing_keys(present: &HashSet<String>) -> Vec<&'static str> {
    let mut missing: Vec<&str> = REQUIRED_KEYS
        .iter()
        .copied()
        .filter(|k| !present.contains(*k))
        .collect();
    if !present.contains("POLYMARKET_WALLET_PRIVATE_KEY")
        && !present.contains("POLYMARKET_PRIVATE_KEY")
    {
        missing.push("POLYMARKET_WALLET_PRIVATE_KEY");
    }
    missing
}

fn live_pid(root: &Path) -> Option<u32> {
    let pid = std::fs::read_to_string(root.join("polybot.pid"))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|p| *p > 1)?;
    pid_is_alive(pid).then_some(pid)
}

fn pid_is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_quiet_hours() -> bool {
    let hour = chrono::Utc::now().hour();
    (0..10).contains(&hour)
}

/// Clear the kill switch and bring the bot up through launchd.
pub async fn start() -> Result<String, String> {
    if !is_enabled() {
        return Err(
            "Polybot is off — turn it on from the Finance tab (risk disclaimer) first".into(),
        );
    }
    let root = require_root()?;
    let paused = root.join("PAUSED");
    if paused.is_file() {
        std::fs::remove_file(&paused).map_err(|e| format!("could not clear PAUSED: {e}"))?;
    }
    ensure_running(&root).await
}

/// Write `PAUSED` so the running loop exits, then ask launchd to stop.
pub fn pause() -> Result<String, String> {
    let root = require_root()?;
    std::fs::write(root.join("PAUSED"), b"").map_err(|e| format!("could not write PAUSED: {e}"))?;
    if let Some(pid) = live_pid(&root) {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
    let _ = launchctl_kill();
    Ok("PAUSED is set — Polybot will not open new positions until you resume it".into())
}

/// Alias for [`start`]: clear `PAUSED` and kick launchd.
pub async fn resume() -> Result<String, String> {
    start().await
}

/// Ask a running bot for a full scan. Starts it if it is down (startup
/// already runs one). Refuses while paused so a kill switch stays a kill
/// switch.
pub async fn request_scan() -> Result<String, String> {
    let root = require_root()?;
    if root.join("PAUSED").is_file() {
        return Err("Polybot is paused — resume it before asking for a scan".into());
    }
    if live_pid(&root).is_some() {
        write_full_scan_trigger(&root)?;
        return Ok(
            "full scan requested — the running loop picks it up within a few seconds. \
             A cycle can take 10–15 minutes; poll polybot_status rather than starting another."
                .into(),
        );
    }
    let started = ensure_running(&root).await?;
    Ok(format!(
        "{started} Startup runs a full scan; bankroll stays stale until that write lands."
    ))
}

fn write_full_scan_trigger(root: &Path) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs_f64();
    let body = serde_json::json!({
        "last_trigger": now,
        "requested_at": now,
    });
    std::fs::write(
        root.join(FULL_SCAN_FILE),
        serde_json::to_vec(&body).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("could not write the scan trigger: {e}"))
}

async fn ensure_running(root: &Path) -> Result<String, String> {
    if let Some(pid) = live_pid(root) {
        return Ok(format!("Polybot is already running (pid {pid})"));
    }
    let creds = credentials_at(root);
    if !creds.ready {
        return Err(creds.detail);
    }
    if !cfg!(target_os = "macos") {
        return Err("starting Polybot is wired for launchd (macOS) only".into());
    }
    let script = root.join("main.py");
    if !script.is_file() {
        return Err(format!(
            "Polybot checkout at {} has no main.py — nothing to start",
            root.display()
        ));
    }
    let python = ensure_python_for_bot(root).await?;

    let home = dirs::home_dir().ok_or("no home directory")?;
    let agents = home.join("Library/LaunchAgents");
    tokio::fs::create_dir_all(&agents)
        .await
        .map_err(|e| e.to_string())?;
    let plist_dst = agents.join(format!("{LAUNCH_LABEL}.plist"));
    let logs = root.join("logs");
    tokio::fs::create_dir_all(&logs)
        .await
        .map_err(|e| e.to_string())?;
    // Keychain is the store. A 0600 sidecar is how a separate Python
    // process receives the values — the plist only points at the path.
    let vault = materialize_runtime_env(root)?;
    let body = launch_plist_body(&python, &script, root, &logs, Some(&vault));
    tokio::fs::write(&plist_dst, body)
        .await
        .map_err(|e| format!("could not write the launch agent: {e}"))?;

    let uid = run("id", &["-u"]).await?;
    let target = format!("gui/{uid}");
    let _ = run(
        "launchctl",
        &["bootstrap", &target, &plist_dst.to_string_lossy()],
    )
    .await;
    run(
        "launchctl",
        &["kickstart", "-k", &format!("{target}/{LAUNCH_LABEL}")],
    )
    .await?;

    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(400)).await;
        if let Some(pid) = live_pid(root) {
            return Ok(format!(
                "Polybot is up (pid {pid}, launchd {}). Startup scans run immediately.",
                plist_dst.display()
            ));
        }
    }
    Ok(format!(
        "asked launchd to start Polybot ({}). If it stays down, see {}/launchd.err.log",
        plist_dst.display(),
        logs.display()
    ))
}

fn launch_plist_body(
    python: &Path,
    script: &Path,
    workdir: &Path,
    logs: &Path,
    vault: Option<&Path>,
) -> String {
    let env = match vault {
        Some(path) => format!(
            "  <key>EnvironmentVariables</key>\n  <dict>\n    <key>POLYBOT_VAULT</key>\n    <string>{}</string>\n  </dict>\n",
            path.display()
        ),
        None => String::new(),
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCH_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>{}</string>
  </array>
  <key>WorkingDirectory</key>
  <string>{}</string>
{env}  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <false/>
  <key>StandardOutPath</key>
  <string>{}</string>
  <key>StandardErrorPath</key>
  <string>{}</string>
</dict>
</plist>
"#,
        python.display(),
        script.display(),
        workdir.display(),
        logs.join("launchd.out.log").display(),
        logs.join("launchd.err.log").display(),
    )
}

/// Write keychain/env secrets to `~/.permagent/runtime/polybot.env` (mode 0600)
/// so launchd can point `POLYBOT_VAULT` at a path that is not the keychain
/// JSON blob and is not the LaunchAgent plist.
fn materialize_runtime_env(root: &Path) -> Result<PathBuf, String> {
    let dest = dirs::home_dir()
        .ok_or("no home directory")?
        .join(".permagent/runtime/polybot.env");
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut lines = Vec::new();
    for key in all_secret_names().chain(["ANTHROPIC_API_KEY", "SLACK_WEBHOOK_URL"]) {
        if let Some(value) = secret_value(key) {
            if value.contains('\n') || value.contains('\r') {
                return Err(format!("{key} contains a newline and cannot be written"));
            }
            lines.push(format!("{key}={value}"));
        }
    }
    if lines.is_empty() {
        // Fall back to an existing vault file the bot already knows how to load.
        if let Some(existing) = vault_candidates(root).into_iter().find(|p| p.is_file()) {
            return Ok(existing);
        }
        return Err("no Polymarket keys in the keychain to hand to Polybot".into());
    }
    write_private(&dest, &(lines.join("\n") + "\n"))?;
    Ok(dest)
}

#[cfg(unix)]
fn write_private(path: &Path, body: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    file.write_all(body.as_bytes())
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn write_private(path: &Path, body: &str) -> Result<(), String> {
    std::fs::write(path, body).map_err(|e| format!("could not write {}: {e}", path.display()))
}

async fn ensure_python_for_bot(root: &Path) -> Result<PathBuf, String> {
    if let Some(py) = python_for_bot(root).await {
        return Ok(py);
    }
    install_bot_venv(root).await?;
    python_for_bot(root).await.ok_or_else(|| {
        format!(
            "installed Polybot deps at {}/.venv but the imports still failed — see that venv",
            root.display()
        )
    })
}

async fn install_bot_venv(root: &Path) -> Result<(), String> {
    let req = root.join("requirements.txt");
    if !req.is_file() {
        return Err(format!(
            "Polybot is at {} but requirements.txt is missing",
            root.display()
        ));
    }
    let venv = root.join(".venv");
    crate::python_runtime::ensure_venv(&venv).await?;
    crate::python_runtime::pip_install(&venv, &["-r", &req.to_string_lossy()]).await
}

async fn python_for_bot(root: &Path) -> Option<PathBuf> {
    let mut candidates = vec![
        root.join(".venv/bin/python"),
        root.join(".venv/bin/python3"),
        root.join("venv/bin/python"),
        root.join("venv/bin/python3"),
    ];
    if let Ok(p) = which_python().await {
        candidates.push(p);
    }
    for py in candidates {
        if py.is_file() && python_imports_bot(&py).await {
            return Some(py);
        }
    }
    None
}

async fn which_python() -> Result<PathBuf, String> {
    let out = run("which", &["python3"]).await?;
    let p = PathBuf::from(out);
    if p.is_file() {
        Ok(p)
    } else {
        Err("python3 not on PATH".into())
    }
}

async fn python_imports_bot(python: &Path) -> bool {
    tokio::process::Command::new(python)
        .args(["-c", "import schedule, py_clob_client"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn run(bin: &str, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new(bin)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("{bin} could not run: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{bin} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn launchctl_kill() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .ok_or("could not read uid")?;
    let _ = std::process::Command::new("launchctl")
        .args(["kill", "SIGTERM", &format!("gui/{uid}/{LAUNCH_LABEL}")])
        .status();
    Ok(())
}

fn require_root() -> Result<PathBuf, String> {
    polybot_root().ok_or_else(|| {
        format!(
            "no Polybot checkout found (looked for polybot under {}). \
             Set polybot_root, then start it.",
            crate::config::dev_roots::dev_roots()
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

fn string_field(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn num_field(v: &serde_json::Value, key: &str) -> Option<f64> {
    v.get(key)
        .and_then(|n| n.as_f64().or_else(|| n.as_i64().map(|i| i as f64)))
}

fn parse_timestamp(ts: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

fn signed_age(ts: &str) -> Option<chrono::TimeDelta> {
    let dt = parse_timestamp(ts)?;
    Some(chrono::Utc::now().signed_duration_since(dt))
}

#[cfg(test)]
fn is_stale(last_updated: Option<&str>) -> bool {
    match last_updated.and_then(signed_age) {
        Some(d) if d.num_seconds() < 0 => false,
        Some(d) => d.to_std().is_ok_and(|std| std > STALE_AFTER),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_checkout(dir: &Path) {
        fs::write(dir.join("main.py"), "print('polybot')\n").unwrap();
        fs::write(dir.join("config.py"), "PAUSED_FILE = 'PAUSED'\n").unwrap();
        fs::create_dir(dir.join("logs")).unwrap();
    }

    #[test]
    fn missing_checkout_is_found_false_not_a_zero_balance() {
        let s = status_from_root(Path::new("/no/such/polybot"));
        assert!(s.current_balance.is_none());
        assert!(s.realized_pnl.is_none());
        assert!(!s.running);
        assert!(!s.credentials_ready);
    }

    #[test]
    fn lowercase_checkout_name_is_found() {
        let home = tempdir().unwrap();
        let root = home.path().join("polybot");
        fs::create_dir(&root).unwrap();
        write_checkout(&root);
        assert_eq!(first_checkout_under(home.path()), Some(root));
    }

    #[test]
    fn a_random_folder_named_polybot_is_ignored() {
        let home = tempdir().unwrap();
        fs::create_dir(home.path().join("polybot")).unwrap();
        assert!(first_checkout_under(home.path()).is_none());
    }

    #[test]
    fn reads_bankroll_and_paused() {
        let dir = tempdir().unwrap();
        write_checkout(dir.path());
        fs::write(dir.path().join("PAUSED"), "").unwrap();
        fs::write(
            dir.path().join("logs/bankroll.json"),
            r#"{
                "current_balance": 81.5,
                "realized_pnl": 8.18,
                "open_exposure": 12.0,
                "trade_count": 14,
                "last_updated": "2099-01-01T00:00:00Z"
            }"#,
        )
        .unwrap();
        let s = status_from_root(dir.path());
        assert!(s.found);
        assert!(s.paused);
        assert_eq!(s.current_balance, Some(81.5));
        assert_eq!(s.realized_pnl, Some(8.18));
        assert_eq!(s.open_exposure, Some(12.0));
        assert_eq!(s.trade_count, Some(14));
        assert!(!s.stale, "a future timestamp is not stale");
        assert!(!s.running);
    }

    #[test]
    fn unreadable_bankroll_is_detail_not_zeros() {
        let dir = tempdir().unwrap();
        write_checkout(dir.path());
        fs::write(dir.path().join("logs/bankroll.json"), "not-json").unwrap();
        let s = status_from_root(dir.path());
        assert!(s.found);
        assert!(s.detail.as_deref().unwrap().contains("unreadable"));
        assert!(s.current_balance.is_none());
    }

    #[test]
    fn a_missing_timestamp_is_stale() {
        assert!(is_stale(None));
        assert!(is_stale(Some("yesterday")));
        assert!(is_stale(Some("2020-01-01T00:00:00Z")));
    }

    /// Live file on 2026-08-22: `last_updated` is Python isoformat with
    /// `+00:00` from 12 May. That must parse and be labelled stale, not shown
    /// as a live wallet.
    #[test]
    fn python_isoformat_from_may_is_stale_with_day_count() {
        let dir = tempdir().unwrap();
        write_checkout(dir.path());
        fs::write(
            dir.path().join("logs/bankroll.json"),
            r#"{
                "current_balance": 100.4046,
                "cash_balance": 100.4046,
                "realized_pnl": -25.3165,
                "open_exposure": 0,
                "trade_count": 527,
                "last_updated": "2026-05-12T18:25:13.814571+00:00",
                "last_wallet_sync": "2026-05-12T15:08:55.939595+00:00"
            }"#,
        )
        .unwrap();
        let s = status_from_root(dir.path());
        assert_eq!(s.current_balance, Some(100.4046));
        assert!(s.stale);
        assert!(
            s.stale_days.unwrap() >= 48,
            "May → August is months: {:?}",
            s.stale_days
        );
        assert!(s.detail.as_deref().unwrap().contains("days ago"));
        assert_eq!(s.as_of.as_deref(), Some("2026-05-12T18:25:13.814571+00:00"));
    }

    #[test]
    fn vault_file_with_required_keys_is_ready_and_values_stay_off_the_status() {
        let _guard = env_lock::lock_env([
            ("POLYMARKET_API_KEY", None::<&str>),
            ("POLYMARKET_API_SECRET", None::<&str>),
            ("POLYMARKET_API_PASSPHRASE", None::<&str>),
            ("POLYMARKET_FUNDER_ADDRESS", None::<&str>),
            ("POLYMARKET_WALLET_PRIVATE_KEY", None::<&str>),
            ("POLYMARKET_PRIVATE_KEY", None::<&str>),
            ("POLYBOT_VAULT", None::<&str>),
        ]);
        let dir = tempdir().unwrap();
        write_checkout(dir.path());
        fs::write(
            dir.path().join("credentials.env"),
            "POLYMARKET_API_KEY=aaa\n\
             POLYMARKET_API_SECRET=bbb\n\
             POLYMARKET_API_PASSPHRASE=ccc\n\
             POLYMARKET_WALLET_PRIVATE_KEY=ddd\n\
             POLYMARKET_FUNDER_ADDRESS=0xabc\n",
        )
        .unwrap();
        let probe = credentials_at(dir.path());
        assert!(probe.ready);
        assert!(probe.path.as_deref().unwrap().ends_with("credentials.env"));
        let s = status_from_root(dir.path());
        assert!(s.credentials_ready);
        assert!(!format!("{s:?}").contains("aaa"));
        assert!(!format!("{s:?}").contains("ddd"));
    }

    #[test]
    fn a_partial_vault_names_the_missing_keys_not_the_values() {
        let _guard = env_lock::lock_env([
            ("POLYMARKET_API_KEY", None::<&str>),
            ("POLYMARKET_API_SECRET", None::<&str>),
            ("POLYMARKET_API_PASSPHRASE", None::<&str>),
            ("POLYMARKET_FUNDER_ADDRESS", None::<&str>),
            ("POLYMARKET_WALLET_PRIVATE_KEY", None::<&str>),
            ("POLYMARKET_PRIVATE_KEY", None::<&str>),
            ("POLYBOT_VAULT", None::<&str>),
        ]);
        let dir = tempdir().unwrap();
        write_checkout(dir.path());
        fs::write(
            dir.path().join("credentials.env"),
            "POLYMARKET_API_KEY=aaa\nPOLYMARKET_API_SECRET=\n",
        )
        .unwrap();
        let probe = credentials_at(dir.path());
        assert!(!probe.ready);
        assert!(probe.detail.contains("POLYMARKET_API_SECRET"));
        assert!(probe.detail.contains("POLYMARKET_FUNDER_ADDRESS"));
        assert!(!probe.detail.contains("aaa"));
    }

    #[test]
    fn scan_trigger_is_the_file_the_bot_already_watches() {
        let dir = tempdir().unwrap();
        write_checkout(dir.path());
        write_full_scan_trigger(dir.path()).unwrap();
        let raw = fs::read_to_string(dir.path().join(FULL_SCAN_FILE)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(v.get("requested_at").and_then(|n| n.as_f64()).unwrap() > 0.0);
        assert!(status_from_root(dir.path()).scan_requested);
    }

    #[test]
    fn missing_keys_point_at_the_keychain_ui() {
        let _guard = env_lock::lock_env([
            ("POLYMARKET_API_KEY", None::<&str>),
            ("POLYMARKET_API_SECRET", None::<&str>),
            ("POLYMARKET_API_PASSPHRASE", None::<&str>),
            ("POLYMARKET_FUNDER_ADDRESS", None::<&str>),
            ("POLYMARKET_WALLET_PRIVATE_KEY", None::<&str>),
            ("POLYMARKET_PRIVATE_KEY", None::<&str>),
            ("POLYBOT_VAULT", None::<&str>),
        ]);
        let dir = tempdir().unwrap();
        write_checkout(dir.path());
        let s = status_from_root(dir.path());
        assert!(!s.credentials_ready);
        assert!(
            s.detail.as_deref().unwrap().contains("keychain"),
            "got {:?}",
            s.detail
        );
    }

    #[test]
    fn generated_plist_points_at_main_and_optional_vault() {
        let body = launch_plist_body(
            Path::new("/usr/bin/python3"),
            Path::new("/tmp/polybot/main.py"),
            Path::new("/tmp/polybot"),
            Path::new("/tmp/polybot/logs"),
            Some(Path::new("/tmp/vault.env")),
        );
        assert!(body.contains(LAUNCH_LABEL));
        assert!(body.contains("/tmp/polybot/main.py"));
        assert!(body.contains("POLYBOT_VAULT"));
        assert!(body.contains("/tmp/vault.env"));
        assert!(
            body.contains("<false/>"),
            "KeepAlive must be off — PAUSED exits"
        );
    }
}
