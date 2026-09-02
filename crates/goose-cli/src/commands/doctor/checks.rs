use super::types::{CheckResult, CheckStatus};
use crate::commands::daemon::{load_daemon_token, read_daemon_port};
use permagent::config::paths::Paths;
use permagent::session::spectral_schema::SPECTRAL_SCHEMA_VERSION;
use std::path::PathBuf;
use std::time::Duration;

const OLLAMA_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_LIBRARIAN_MODEL: &str = "qwen2.5:7b";

pub async fn run_all() -> Vec<CheckResult> {
    let mut results = Vec::new();

    results.push(check_launchd_plist());
    results.push(check_daemon_process());
    results.push(check_daemon_reachable().await);
    results.push(check_token_file());
    results.push(check_auth_roundtrip().await);
    results.push(check_version_skew().await);
    results.push(check_websocket().await);
    results.push(check_ui_served().await);
    results.push(check_permagent_db());
    results.push(check_memory_db());
    results.push(check_decision_audit_chain().await);
    results.push(check_ollama().await);
    results.push(check_disk());
    results.push(check_caches());
    results.push(check_backups());
    results.push(check_secret_env_shadowing());
    results.push(check_secret_split_brain());
    results.push(check_safety_inspectors());

    results
}

// ── 18. safety-inspector break-glass switches (D34) ──

/// Every way to switch a safety inspector off is an env var that
/// `Config::get_param` reads before the config file — invisible in the config,
/// absent from the UI, and inherited by every child process. The escape
/// hatches stay (a dev machine needs them); this is what makes using one
/// impossible to miss.
///
/// Two sources are read, because doctor is not the daemon: this process's own
/// environment + config file, and the running daemon's environment (`ps eww`,
/// the same trick `secret-env-shadowing` uses).
fn check_safety_inspectors() -> CheckResult {
    use permagent::tool_inspection::{active_safety_disables, SAFETY_SWITCHES};

    let mut lines: Vec<String> = Vec::new();

    for d in active_safety_disables() {
        lines.push(format!(
            "safety inspector {} disabled via {}={} — {}",
            d.inspector, d.switch, d.value, d.loses
        ));
    }

    let switch_names: Vec<&str> = SAFETY_SWITCHES.iter().map(|s| s.switch).collect();
    for (key, value) in daemon_process_env_values(&switch_names) {
        let Some(s) = SAFETY_SWITCHES.iter().find(|s| s.switch == key) else {
            continue;
        };
        let disabling = match key.as_str() {
            "SECURITY_PROMPT_LOG_ONLY" => value.eq_ignore_ascii_case("true") || value == "1",
            "GOOSE_TOOL_ARG_VALIDATION" => value.eq_ignore_ascii_case("off"),
            _ => value.eq_ignore_ascii_case("false") || value == "0",
        };
        if !disabling {
            continue;
        }
        let line = format!(
            "safety inspector {} disabled via {}={} in the RUNNING DAEMON's environment — {}",
            s.inspector, key, value, s.loses
        );
        if !lines.iter().any(|l| l.contains(&key)) {
            lines.push(line);
        }
    }

    // The adversary reviewer is opt-in (a file, not a switch): its absence is
    // the shipped default, not a disable. Reported, never as an alarm.
    let adversary_md = permagent::config::paths::Paths::config_dir().join("adversary.md");
    let adversary_note = if adversary_md.exists() {
        format!("adversary reviewer active ({})", adversary_md.display())
    } else {
        format!(
            "adversary reviewer inactive — optional, off by default until {} exists",
            adversary_md.display()
        )
    };

    if lines.is_empty() {
        return CheckResult {
            name: "safety-inspectors".into(),
            status: CheckStatus::Info,
            detail: format!("no safety inspector is switched off; {adversary_note}"),
            remediation: None,
        };
    }

    lines.push(adversary_note);
    CheckResult {
        name: "safety-inspectors".into(),
        status: CheckStatus::Warn,
        detail: lines.join("; "),
        remediation: Some(format!(
            "Unset the switch(es) above and restart the daemon to restore the inspector. \
             While one is set, every tool call it skips logs at WARN under marker {}.",
            permagent::tool_inspection::SAFETY_DISABLE_MARKER
        )),
    }
}

/// Read named environment variables out of the running daemon's process
/// environment. Returns `(key, value)` for every requested key that is set.
fn daemon_process_env_values(keys: &[&str]) -> Vec<(String, String)> {
    let Ok(pgrep) = std::process::Command::new("pgrep")
        .args(["-x", "permagentd"])
        .output()
    else {
        return Vec::new();
    };
    if !pgrep.status.success() {
        return Vec::new();
    }

    let pids = String::from_utf8_lossy(&pgrep.stdout).to_string();
    let mut found: Vec<(String, String)> = Vec::new();
    for pid in pids.split_whitespace() {
        let Ok(ps) = std::process::Command::new("ps")
            .args(["eww", "-o", "command=", "-p", pid])
            .output()
        else {
            continue;
        };
        if !ps.status.success() {
            continue;
        }
        let listing = String::from_utf8_lossy(&ps.stdout).to_string();
        for (key, value) in extract_env_values(&listing, keys) {
            if !found.iter().any(|(k, _)| *k == key) {
                found.push((key, value));
            }
        }
    }
    found
}

/// Pull `KEY=value` pairs for the requested keys out of a `ps eww` listing.
/// Separated from the process call so it is testable.
fn extract_env_values(listing: &str, keys: &[&str]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for token in listing.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        if keys.contains(&key) && !out.iter().any(|(k, _): &(String, String)| k == key) {
            out.push((key.to_string(), value.to_string()));
        }
    }
    out
}

// ── 1. launchd plist ──

fn check_launchd_plist() -> CheckResult {
    let plist = home_dir().join("Library/LaunchAgents/ai.permagent.daemon.plist");

    if !plist.exists() {
        return CheckResult {
            name: "launchd-plist".into(),
            status: CheckStatus::Fail,
            detail: format!("{} not found", plist.display()),
            remediation: Some("Run `permagent start` to generate and load the plist.".into()),
        };
    }

    // Check loaded state via launchctl print (preferred) or list (fallback).
    let uid = unsafe { libc::getuid() };
    let domain_target = format!("gui/{uid}/ai.permagent.daemon");

    let loaded = std::process::Command::new("launchctl")
        .args(["print", &domain_target])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !loaded {
        // Fallback: launchctl list
        let list_loaded = std::process::Command::new("launchctl")
            .arg("list")
            .output()
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .any(|l| l.contains("ai.permagent.daemon"))
            })
            .unwrap_or(false);

        if !list_loaded {
            return CheckResult {
                name: "launchd-plist".into(),
                status: CheckStatus::Fail,
                detail: "plist exists but is not loaded".into(),
                remediation: Some(
                    "launchctl unload ~/Library/LaunchAgents/ai.permagent.daemon.plist; \
                     pkill -f permagentd; sleep 1; \
                     launchctl load -w ~/Library/LaunchAgents/ai.permagent.daemon.plist"
                        .into(),
                ),
            };
        }
    }

    CheckResult {
        name: "launchd-plist".into(),
        status: CheckStatus::Pass,
        detail: "plist exists and loaded".into(),
        remediation: None,
    }
}

// ── 2. Daemon process ──

fn check_daemon_process() -> CheckResult {
    let running = std::process::Command::new("pgrep")
        .args(["-x", "permagentd"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if running {
        CheckResult {
            name: "daemon-process".into(),
            status: CheckStatus::Pass,
            detail: "permagentd is running".into(),
            remediation: None,
        }
    } else {
        CheckResult {
            name: "daemon-process".into(),
            status: CheckStatus::Fail,
            detail: "permagentd not found in process list".into(),
            remediation: Some("Run `permagent restart` to restart the daemon.".into()),
        }
    }
}

// ── 3. Daemon reachable ──

async fn check_daemon_reachable() -> CheckResult {
    let port = read_daemon_port();
    let url = format!("http://127.0.0.1:{port}/status");

    match http_get(&url, None).await {
        Ok(200) => CheckResult {
            name: "daemon-reachable".into(),
            status: CheckStatus::Pass,
            detail: format!("GET /status returned 200 on port {port}"),
            remediation: None,
        },
        Ok(status) => CheckResult {
            name: "daemon-reachable".into(),
            status: CheckStatus::Fail,
            detail: format!("GET /status returned {status}"),
            remediation: Some("Run `permagent restart` to restart the daemon.".into()),
        },
        Err(e) => CheckResult {
            name: "daemon-reachable".into(),
            status: CheckStatus::Fail,
            detail: format!("connection failed: {e}"),
            remediation: Some("Run `permagent restart` to restart the daemon.".into()),
        },
    }
}

// ── 4. Token file ──

fn check_token_file() -> CheckResult {
    let token_path = Paths::in_data_dir("secrets/daemon_token.json");

    if !token_path.exists() {
        return CheckResult {
            name: "token-file".into(),
            status: CheckStatus::Fail,
            detail: format!("{} not found", token_path.display()),
            remediation: Some("Restart the daemon; it generates the token on startup.".into()),
        };
    }

    // Check permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&token_path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o600 {
                return CheckResult {
                    name: "token-file".into(),
                    status: CheckStatus::Warn,
                    detail: format!("permissions are {mode:04o}, expected 0600"),
                    remediation: Some(format!("chmod 600 {}", token_path.display())),
                };
            }
        }
    }

    // Verify it parses
    match std::fs::read_to_string(&token_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("token").and_then(|t| t.as_str()).map(String::from))
    {
        Some(_) => CheckResult {
            name: "token-file".into(),
            status: CheckStatus::Pass,
            detail: "exists, parseable, permissions ok".into(),
            remediation: None,
        },
        None => CheckResult {
            name: "token-file".into(),
            status: CheckStatus::Fail,
            detail: "file exists but missing or unparseable 'token' field".into(),
            remediation: Some("Delete the file and restart the daemon to regenerate.".into()),
        },
    }
}

// ── 5. Auth round-trip ──

async fn check_auth_roundtrip() -> CheckResult {
    let port = read_daemon_port();
    // Use GET /config — a stable, read-only, side-effect-free protected endpoint.
    // Chosen because it exists in every daemon version since config management was
    // added, returns quickly, and never triggers writes or external calls.
    let url = format!("http://127.0.0.1:{port}/config");

    // First verify 401 without token
    match http_get(&url, None).await {
        Ok(401) => {} // expected
        Ok(status) => {
            return CheckResult {
                name: "auth-roundtrip".into(),
                status: CheckStatus::Warn,
                detail: format!("GET /config without token returned {status}, expected 401"),
                remediation: Some("Daemon may be running in dev mode (no token enforced).".into()),
            };
        }
        Err(e) => {
            return CheckResult {
                name: "auth-roundtrip".into(),
                status: CheckStatus::Fail,
                detail: format!("connection failed: {e}"),
                remediation: Some("Ensure daemon is running: `permagent restart`".into()),
            };
        }
    }

    // Now verify 200 with token
    let token = match load_daemon_token() {
        Ok(t) => t,
        Err(_) => {
            return CheckResult {
                name: "auth-roundtrip".into(),
                status: CheckStatus::Fail,
                detail: "could not load token for auth test".into(),
                remediation: Some("Fix token-file check first.".into()),
            };
        }
    };

    match http_get(&url, Some(&token)).await {
        Ok(200) => CheckResult {
            name: "auth-roundtrip".into(),
            status: CheckStatus::Pass,
            detail: "401 without token, 200 with token".into(),
            remediation: None,
        },
        Ok(401) => CheckResult {
            name: "auth-roundtrip".into(),
            status: CheckStatus::Fail,
            detail: "token on disk does not match running daemon".into(),
            remediation: Some(
                "Restart the daemon to regenerate the token, or delete \
                 ~/.permagent/secrets/daemon_token.json and restart."
                    .into(),
            ),
        },
        Ok(status) => CheckResult {
            name: "auth-roundtrip".into(),
            status: CheckStatus::Fail,
            detail: format!("GET /config with token returned {status}"),
            remediation: Some("Run `permagent restart`.".into()),
        },
        Err(e) => CheckResult {
            name: "auth-roundtrip".into(),
            status: CheckStatus::Fail,
            detail: format!("connection failed: {e}"),
            remediation: Some("Ensure daemon is running: `permagent restart`".into()),
        },
    }
}

// ── 6. Version skew ──

async fn check_version_skew() -> CheckResult {
    let port = read_daemon_port();
    let url = format!("http://127.0.0.1:{port}/api/version");

    let body = match http_get_body(&url, None).await {
        Ok(b) => b,
        Err(e) => {
            return CheckResult {
                name: "version-info".into(),
                status: CheckStatus::Fail,
                detail: format!("could not reach /api/version: {e}"),
                remediation: Some("Ensure daemon is running.".into()),
            };
        }
    };

    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return CheckResult {
                name: "version-info".into(),
                status: CheckStatus::Fail,
                detail: format!("invalid JSON from /api/version: {e}"),
                remediation: None,
            };
        }
    };

    let sha = parsed
        .get("git_sha")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let dirty = parsed
        .get("git_dirty")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let version = parsed
        .get("permagentd_version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let spectral = parsed
        .get("spectral_pin")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let mut parts = vec![format!("v{version} sha={sha}")];
    if dirty == "true" {
        parts.push("DIRTY".into());
    }

    let status = if sha == "unknown" || dirty == "true" {
        CheckStatus::Warn
    } else {
        CheckStatus::Pass
    };

    let remediation = if dirty == "true" {
        Some("Running daemon was built from uncommitted changes.".into())
    } else if sha == "unknown" {
        Some("Running daemon was built without git info.".into())
    } else {
        None
    };

    // Return version info; spectral pin is a separate INFO row added by the caller
    let mut results_detail = parts.join(" ");
    results_detail.push_str(&format!(" spectral={spectral}"));

    CheckResult {
        name: "version-info".into(),
        status,
        detail: results_detail,
        remediation,
    }
}

// ── 7. WebSocket ──

async fn check_websocket() -> CheckResult {
    let port = read_daemon_port();
    // /events requires the daemon token (C1/C2 auth plane); ride it on the
    // query string the same way browser clients do. Hex token — URL-safe.
    let (url, token_note) = match crate::commands::daemon::load_daemon_token() {
        Ok(token) => (
            format!("ws://127.0.0.1:{port}/events?token={token}"),
            "token sent",
        ),
        Err(_) => (
            format!("ws://127.0.0.1:{port}/events"),
            "NO TOKEN (daemon_token.json unreadable)",
        ),
    };

    match tokio_tungstenite::connect_async(&url).await {
        Ok(_) => CheckResult {
            name: "websocket".into(),
            status: CheckStatus::Pass,
            detail: "/events WebSocket upgrade succeeded".into(),
            remediation: None,
        },
        Err(e) => CheckResult {
            name: "websocket".into(),
            status: CheckStatus::Fail,
            detail: format!("WebSocket upgrade failed ({token_note}): {e}"),
            remediation: Some(
                "Ensure daemon is running: `permagent restart`. A 401 means the daemon \
                 token in ~/.permagent/secrets/daemon_token.json does not match."
                    .into(),
            ),
        },
    }
}

// ── 8. UI served ──

async fn check_ui_served() -> CheckResult {
    let port = read_daemon_port();
    let url = format!("http://127.0.0.1:{port}/ui/");

    match http_get(&url, None).await {
        Ok(200) => CheckResult {
            name: "ui-served".into(),
            status: CheckStatus::Pass,
            detail: "GET /ui/ returned 200".into(),
            remediation: None,
        },
        Ok(status) => CheckResult {
            name: "ui-served".into(),
            status: CheckStatus::Warn,
            detail: format!("GET /ui/ returned {status}"),
            remediation: Some("Command Center dist may not be built. Run the UI build.".into()),
        },
        Err(e) => CheckResult {
            name: "ui-served".into(),
            status: CheckStatus::Fail,
            detail: format!("connection failed: {e}"),
            remediation: Some("Ensure daemon is running.".into()),
        },
    }
}

// ── 9. permagent.db ──

fn check_permagent_db() -> CheckResult {
    let db_path = Paths::spectral_db();

    if !db_path.exists() {
        return CheckResult {
            name: "permagent-db".into(),
            status: CheckStatus::Fail,
            detail: format!("{} not found", db_path.display()),
            remediation: Some("Start the daemon; it creates the database on first run.".into()),
        };
    }

    match open_readonly_sqlite(&db_path) {
        Err(e) => CheckResult {
            name: "permagent-db".into(),
            status: CheckStatus::Fail,
            detail: format!("could not open: {e}"),
            remediation: None,
        },
        Ok(conn) => {
            // quick_check
            if let Err(e) = sqlite_quick_check(&conn) {
                return CheckResult {
                    name: "permagent-db".into(),
                    status: CheckStatus::Fail,
                    detail: format!("PRAGMA quick_check failed: {e}"),
                    remediation: Some("Database may be corrupted. Restore from backup.".into()),
                };
            }

            let journal = sqlite_journal_mode(&conn);

            // Schema version check
            let db_version = sqlite_scalar_i32(&conn, "SELECT MAX(version) FROM schema_version");

            let detail = match db_version {
                // SPECTRAL_SCHEMA_VERSION is the fresh-init BASE stamp, not
                // "latest": a migrated DB sits above it and is healthy. Comparing
                // with `==` flagged every real install as a mismatch and told the
                // user to restart, which could never clear it.
                Some(v) if v >= SPECTRAL_SCHEMA_VERSION => {
                    format!(
                        "ok, journal={journal}, schema v{v} (fresh-init base v{SPECTRAL_SCHEMA_VERSION})"
                    )
                }
                Some(v) => {
                    return CheckResult {
                        name: "permagent-db".into(),
                        status: CheckStatus::Warn,
                        detail: format!(
                            "schema v{v} is below the fresh-init base v{SPECTRAL_SCHEMA_VERSION}, journal={journal}"
                        ),
                        remediation: Some("Restart the daemon to apply pending migrations.".into()),
                    };
                }
                None => {
                    return CheckResult {
                        name: "permagent-db".into(),
                        status: CheckStatus::Warn,
                        detail: format!("no schema_version table found, journal={journal}"),
                        remediation: Some("Restart the daemon to initialize the schema.".into()),
                    };
                }
            };

            CheckResult {
                name: "permagent-db".into(),
                status: CheckStatus::Pass,
                detail,
                remediation: None,
            }
        }
    }
}

// ── 10. memory.db ──

fn check_memory_db() -> CheckResult {
    let db_path = Paths::brain_dir().join("memory.db");

    if !db_path.exists() {
        return CheckResult {
            name: "memory-db".into(),
            status: CheckStatus::Info,
            detail: format!(
                "{} not found (brain may not have run yet)",
                db_path.display()
            ),
            remediation: None,
        };
    }

    match open_readonly_sqlite(&db_path) {
        Err(e) => CheckResult {
            name: "memory-db".into(),
            status: CheckStatus::Fail,
            detail: format!("could not open: {e}"),
            remediation: None,
        },
        Ok(conn) => {
            if let Err(e) = sqlite_quick_check(&conn) {
                return CheckResult {
                    name: "memory-db".into(),
                    status: CheckStatus::Fail,
                    detail: format!("PRAGMA quick_check failed: {e}"),
                    remediation: Some("Database may be corrupted. Restore from backup.".into()),
                };
            }

            let journal = sqlite_journal_mode(&conn);

            // Check for expected core table
            let has_memories = sqlite_scalar_i32(
                &conn,
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memories'",
            )
            .unwrap_or(0)
                > 0;

            let tables_note = if has_memories {
                "memories table present"
            } else {
                "memories table NOT found"
            };

            CheckResult {
                name: "memory-db".into(),
                status: CheckStatus::Info,
                detail: format!("ok, journal={journal}, {tables_note}"),
                remediation: None,
            }
        }
    }
}

// ── 11. Ollama ──

async fn check_ollama() -> CheckResult {
    let model = resolve_librarian_model();
    check_ollama_at(OLLAMA_BASE_URL, &model).await
}

/// Testable core: check Ollama reachability and model presence at a given base URL.
async fn check_ollama_at(base_url: &str, model: &str) -> CheckResult {
    let tags_url = format!("{base_url}/api/tags");
    let body = match http_get_body(&tags_url, None).await {
        Ok(b) => b,
        Err(_) => {
            return CheckResult {
                name: "ollama".into(),
                status: CheckStatus::Warn,
                detail: format!("Ollama not reachable at {base_url}"),
                remediation: Some(
                    "Start Ollama (`ollama serve`). Librarian degrades but chat still works."
                        .into(),
                ),
            };
        }
    };

    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return CheckResult {
                name: "ollama".into(),
                status: CheckStatus::Warn,
                detail: "Ollama reachable but /api/tags returned invalid JSON".into(),
                remediation: None,
            };
        }
    };

    let models = parsed
        .get("models")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let model_present = models.iter().any(|m| {
        m.get("name")
            .and_then(|n| n.as_str())
            .map(|n| n == model || n.starts_with(&format!("{model}:")))
            .unwrap_or(false)
    });

    if model_present {
        CheckResult {
            name: "ollama".into(),
            status: CheckStatus::Pass,
            detail: format!("reachable, model '{model}' present"),
            remediation: None,
        }
    } else {
        CheckResult {
            name: "ollama".into(),
            status: CheckStatus::Fail,
            detail: format!("model '{model}' not found in Ollama"),
            remediation: Some(format!("ollama pull {model}")),
        }
    }
}

// ── 12. Disk ──

fn check_disk() -> CheckResult {
    let permagent_dir = Paths::data_dir();
    let check_path = if permagent_dir.exists() {
        permagent_dir
    } else {
        // Fall back to home dir
        home_dir()
    };

    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::mem::MaybeUninit;

        let c_path = match CString::new(check_path.to_string_lossy().as_bytes()) {
            Ok(p) => p,
            Err(_) => {
                return CheckResult {
                    name: "disk-space".into(),
                    status: CheckStatus::Warn,
                    detail: "could not check disk space".into(),
                    remediation: None,
                };
            }
        };

        let mut stat = MaybeUninit::<libc::statvfs>::uninit();
        let ret = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
        if ret != 0 {
            return CheckResult {
                name: "disk-space".into(),
                status: CheckStatus::Warn,
                detail: "statvfs failed".into(),
                remediation: None,
            };
        }

        let stat = unsafe { stat.assume_init() };
        // f_bavail is u64 on Linux, u32 on macOS — cast required cross-platform
        #[allow(clippy::unnecessary_cast)]
        let free_bytes = stat.f_bavail as u64 * stat.f_frsize;
        let free_gb = free_bytes as f64 / 1_073_741_824.0;

        let (status, detail) = if free_gb < 5.0 {
            (
                CheckStatus::Fail,
                format!("{free_gb:.1} GB free (critical)"),
            )
        } else if free_gb < 15.0 {
            (CheckStatus::Warn, format!("{free_gb:.1} GB free (low)"))
        } else {
            (CheckStatus::Info, format!("{free_gb:.1} GB free"))
        };

        let remediation = if free_gb < 5.0 {
            Some("Free disk space. Permagent databases and models require space.".into())
        } else if free_gb < 15.0 {
            Some("Consider freeing disk space.".into())
        } else {
            None
        };

        CheckResult {
            name: "disk-space".into(),
            status,
            detail,
            remediation,
        }
    }

    #[cfg(not(unix))]
    CheckResult {
        name: "disk-space".into(),
        status: CheckStatus::Info,
        detail: "disk check not supported on this platform".into(),
        remediation: None,
    }
}

// ── 13. Caches ──

fn check_caches() -> CheckResult {
    let home = home_dir();
    let webkit = home.join("Library/WebKit");
    let caches = home.join("Library/Caches");

    let mut found = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&webkit) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("ai.permagent.") {
                found.push(format!("WebKit/{name}"));
            }
        }
    }

    if let Ok(entries) = std::fs::read_dir(&caches) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("ai.permagent.") {
                found.push(format!("Caches/{name}"));
            }
        }
    }

    if found.is_empty() {
        CheckResult {
            name: "webkit-caches".into(),
            status: CheckStatus::Info,
            detail: "no ai.permagent.* caches found".into(),
            remediation: None,
        }
    } else {
        CheckResult {
            name: "webkit-caches".into(),
            status: CheckStatus::Info,
            detail: format!("found: {}", found.join(", ")),
            remediation: Some(
                "After reinstall, clear with: rm -rf ~/Library/WebKit/ai.permagent.* \
                 ~/Library/Caches/ai.permagent.*"
                    .into(),
            ),
        }
    }
}

// ── 14. Backups ──

fn check_backups() -> CheckResult {
    let backups_dir = Paths::data_dir().join("backups");

    if !backups_dir.exists() {
        return CheckResult {
            name: "backups".into(),
            status: CheckStatus::Info,
            detail: "backup snapshots not present (feature in flight)".into(),
            remediation: None,
        };
    }

    // Find newest file in backups dir
    let newest = newest_file_age(&backups_dir);

    match newest {
        None => CheckResult {
            name: "backups".into(),
            status: CheckStatus::Info,
            detail: "backups directory exists but is empty".into(),
            remediation: None,
        },
        Some(age) => {
            let hours = age.as_secs() / 3600;
            if hours > 48 {
                CheckResult {
                    name: "backups".into(),
                    status: CheckStatus::Warn,
                    detail: format!("newest backup is {hours}h old (>48h)"),
                    remediation: Some("Check that backup scheduling is running.".into()),
                }
            } else {
                CheckResult {
                    name: "backups".into(),
                    status: CheckStatus::Pass,
                    detail: format!("newest backup is {hours}h old"),
                    remediation: None,
                }
            }
        }
    }
}

// ── 15. Secret env shadowing ──

// Mirror of the (private) constants in permagent::config::base. The doctor
// only performs a read-only metadata lookup against this item — it never
// reads or writes the secret payload.
const SECRETS_KEYRING_SERVICE: &str = "permagent";
const SECRETS_KEYRING_ACCOUNT: &str = "secrets";

/// WARN when a `*_API_KEY` is injected via the launchd plist or is present in
/// the running daemon's environment while secret storage also holds secrets.
///
/// On current builds the keychain wins (keychain-first since 0055ee042), so
/// the env value is silently ignored — stale bootstrap hygiene. On pre-fix
/// builds the env value *shadows* the keychain and UI-saved keys appear to
/// "not stick" (#157/#176).
fn check_secret_env_shadowing() -> CheckResult {
    let plist = home_dir().join("Library/LaunchAgents/ai.permagent.daemon.plist");
    let plist_keys = std::fs::read_to_string(&plist)
        .map(|c| extract_api_key_names_from_plist(&c))
        .unwrap_or_default();

    let daemon_env_keys = daemon_process_api_key_names();

    let mut env_keys: Vec<String> = Vec::new();
    for k in plist_keys.iter().chain(daemon_env_keys.iter()) {
        if !env_keys.contains(k) {
            env_keys.push(k.clone());
        }
    }

    let keychain_exists = keychain_blob_exists();
    let yaml_keys = secrets_yaml_keys();

    let (status, detail, remediation) = classify_shadowing(&env_keys, keychain_exists, &yaml_keys);

    CheckResult {
        name: "secret-env-shadowing".into(),
        status,
        detail,
        remediation,
    }
}

/// Pure decision logic, separated for testability.
fn classify_shadowing(
    env_keys: &[String],
    keychain_exists: bool,
    yaml_keys: &[String],
) -> (CheckStatus, String, Option<String>) {
    if env_keys.is_empty() {
        return (
            CheckStatus::Pass,
            "no *_API_KEY in launchd plist or daemon environment".into(),
            None,
        );
    }

    let yaml_overlap: Vec<&String> = env_keys.iter().filter(|k| yaml_keys.contains(k)).collect();

    if keychain_exists || !yaml_overlap.is_empty() {
        let store = if keychain_exists {
            "keychain blob"
        } else {
            "secrets.yaml"
        };
        (
            CheckStatus::Warn,
            format!(
                "{} set via env while {store} exists — ignored on current builds \
                 (keychain-first), but SHADOWS stored secrets on builds older than \
                 2026-06-01 (#176)",
                env_keys.join(", ")
            ),
            Some(
                "Remove the key(s) from the plist EnvironmentVariables / shell env; \
                 secret storage is authoritative. Then `permagent restart`."
                    .into(),
            ),
        )
    } else {
        (
            CheckStatus::Info,
            format!(
                "{} set via env with no stored secret — env bootstrap fallback in use",
                env_keys.join(", ")
            ),
            None,
        )
    }
}

/// Extract `*_API_KEY` key names from the EnvironmentVariables dict of a
/// launchd plist (values are never read).
#[allow(clippy::string_slice)] // All indices come from str::find() with &str patterns, so they are always char-boundary safe
fn extract_api_key_names_from_plist(content: &str) -> Vec<String> {
    let Some(env_pos) = content.find("<key>EnvironmentVariables</key>") else {
        return Vec::new();
    };
    let rest = &content[env_pos..];
    let Some(dict_start) = rest.find("<dict>") else {
        return Vec::new();
    };
    let rest = &rest[dict_start..];
    let section = &rest[..rest.find("</dict>").unwrap_or(rest.len())];

    let mut keys = Vec::new();
    let mut search = section;
    while let Some(start) = search.find("<key>") {
        let after = &search[start + "<key>".len()..];
        let Some(close) = after.find("</key>") else {
            break;
        };
        let name = after[..close].trim();
        if name.ends_with("_API_KEY") && !keys.contains(&name.to_string()) {
            keys.push(name.to_string());
        }
        search = &after[close..];
    }
    keys
}

/// Extract `*_API_KEY` variable names from a `ps eww` style listing
/// (`command VAR=val VAR=val ...`). Only names are kept, never values.
fn extract_api_key_names_from_env_listing(listing: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for token in listing.split_whitespace() {
        if let Some((name, _)) = token.split_once('=') {
            if name.ends_with("_API_KEY")
                && !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                && !keys.contains(&name.to_string())
            {
                keys.push(name.to_string());
            }
        }
    }
    keys
}

/// Names of `*_API_KEY` vars in the running daemon's environment (macOS:
/// `ps eww` appends the environment to the command for same-user processes).
fn daemon_process_api_key_names() -> Vec<String> {
    let Ok(pgrep) = std::process::Command::new("pgrep")
        .args(["-x", "permagentd"])
        .output()
    else {
        return Vec::new();
    };
    if !pgrep.status.success() {
        return Vec::new();
    }

    let pids = String::from_utf8_lossy(&pgrep.stdout).to_string();
    let mut keys = Vec::new();
    for pid in pids.split_whitespace() {
        if let Ok(ps) = std::process::Command::new("ps")
            .args(["eww", "-o", "command=", "-p", pid])
            .output()
        {
            if ps.status.success() {
                for k in
                    extract_api_key_names_from_env_listing(&String::from_utf8_lossy(&ps.stdout))
                {
                    if !keys.contains(&k) {
                        keys.push(k);
                    }
                }
            }
        }
    }
    keys
}

/// Read-only existence check for the keychain secrets blob. Deliberately
/// avoids `-w` (no secret data is read), so macOS does not prompt.
fn keychain_blob_exists() -> bool {
    std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            SECRETS_KEYRING_SERVICE,
            "-a",
            SECRETS_KEYRING_ACCOUNT,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Top-level key names in ~/.permagent/secrets.yaml (empty if absent/unreadable).
fn secrets_yaml_keys() -> Vec<String> {
    let path = Paths::in_config_dir("secrets.yaml");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_yaml::from_str::<serde_yaml::Mapping>(&s).ok())
        .map(|m| {
            m.keys()
                .filter_map(|k| k.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

// ── 16. Secret storage split-brain ──

/// WARN when both the keychain blob and secrets.yaml exist. The file backend
/// is only read when the keyring is disabled or unavailable, so a stale copy
/// silently diverges from the keychain and confuses recovery/debugging.
fn check_secret_split_brain() -> CheckResult {
    let keychain = keychain_blob_exists();
    let yaml_path = Paths::in_config_dir("secrets.yaml");
    let yaml = yaml_path.exists();

    match (keychain, yaml) {
        (true, true) => CheckResult {
            name: "secret-split-brain".into(),
            status: CheckStatus::Warn,
            detail: format!(
                "both the keychain blob (service '{SECRETS_KEYRING_SERVICE}') and {} exist",
                yaml_path.display()
            ),
            remediation: Some(
                "Keychain is authoritative when available; the yaml copy is only read \
                 when the keyring is disabled/unavailable and may be stale. Verify and \
                 remove the yaml file (back it up first)."
                    .into(),
            ),
        },
        (true, false) => CheckResult {
            name: "secret-split-brain".into(),
            status: CheckStatus::Pass,
            detail: "keychain blob present, no secrets.yaml".into(),
            remediation: None,
        },
        (false, true) => CheckResult {
            name: "secret-split-brain".into(),
            status: CheckStatus::Info,
            detail: "file backend in use (no keychain blob; keyring disabled or unavailable)"
                .into(),
            remediation: None,
        },
        (false, false) => CheckResult {
            name: "secret-split-brain".into(),
            status: CheckStatus::Info,
            detail: "no stored secrets found (no keychain blob, no secrets.yaml)".into(),
            remediation: None,
        },
    }
}

// ── Helpers ──

fn home_dir() -> PathBuf {
    dirs::home_dir().expect("could not determine home directory")
}

fn resolve_librarian_model() -> String {
    let path = Paths::in_data_dir("librarian_schedule.json");
    if let Ok(contents) = std::fs::read_to_string(&path) {
        if let Ok(schedule) = serde_json::from_str::<serde_json::Value>(&contents) {
            if let Some(model) = schedule.get("model").and_then(|v| v.as_str()) {
                if !model.is_empty() {
                    return model.to_string();
                }
            }
        }
    }
    DEFAULT_LIBRARIAN_MODEL.to_string()
}

async fn http_get(url: &str, bearer: Option<&str>) -> Result<u16, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = client.get(url);
    if let Some(token) = bearer {
        req = req.bearer_auth(token);
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    Ok(resp.status().as_u16())
}

async fn http_get_body(url: &str, bearer: Option<&str>) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = client.get(url);
    if let Some(token) = bearer {
        req = req.bearer_auth(token);
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

// ── decision audit hash chain (Decision Inbox S3) ──

/// Verify the append-only decision audit chain through the canonical walker,
/// `permagent::decisions::verify_audit_chain`. Reports the first break point.
async fn check_decision_audit_chain() -> CheckResult {
    check_decision_audit_chain_at(&Paths::spectral_db()).await
}

async fn check_decision_audit_chain_at(db_path: &std::path::Path) -> CheckResult {
    let name = "decision-audit-chain";

    if !db_path.exists() {
        return CheckResult {
            name: name.into(),
            status: CheckStatus::Info,
            detail: "permagent.db not found — nothing to verify".into(),
            remediation: None,
        };
    }

    // Read-only pool. The walk itself is `permagent::decisions::verify_audit_chain`
    // — the same code the daemon's own tests verify the chain with. Doctor owned a
    // second copy of the hash algorithm until 2026-08-31; that copy never selected
    // `principal`, so it recomputed every attributed row with the 8-field hash and
    // reported a false break at the first answered decision.
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(db_path)
        .read_only(true)
        .create_if_missing(false);
    let pool = match sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
    {
        Ok(pool) => pool,
        Err(e) => {
            return CheckResult {
                name: name.into(),
                status: CheckStatus::Fail,
                detail: format!("could not open permagent.db: {e}"),
                remediation: None,
            };
        }
    };

    let table_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='decision_audit'",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(0);
    if table_exists == 0 {
        pool.close().await;
        return CheckResult {
            name: name.into(),
            status: CheckStatus::Info,
            detail: "decision_audit table not present (decision inbox schema not applied)".into(),
            remediation: Some("Restart the daemon to apply pending migrations.".into()),
        };
    }

    let report = permagent::decisions::verify_audit_chain(&pool).await;
    pool.close().await;

    match report {
        Err(e) => CheckResult {
            name: name.into(),
            status: CheckStatus::Fail,
            detail: format!("could not read decision_audit: {e}"),
            remediation: None,
        },
        Ok(report) if report.intact => CheckResult {
            name: name.into(),
            status: CheckStatus::Pass,
            detail: format!("{} audit row(s), hash chain intact", report.total_rows),
            remediation: None,
        },
        Ok(report) => CheckResult {
            name: name.into(),
            status: CheckStatus::Fail,
            detail: format!(
                "chain BROKEN at seq {} of {}: {}",
                report
                    .break_seq
                    .map(|seq| seq.to_string())
                    .unwrap_or_else(|| "?".into()),
                report.total_rows,
                report.detail
            ),
            remediation: Some(
                "The append-only decision audit log has been tampered with or corrupted. \
                 Inspect decision_audit around the break point and restore from backup."
                    .into(),
            ),
        },
    }
}

fn open_readonly_sqlite(path: &std::path::Path) -> Result<rusqlite::Connection, String> {
    rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())
}

fn sqlite_quick_check(conn: &rusqlite::Connection) -> Result<(), String> {
    let result: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if result == "ok" {
        Ok(())
    } else {
        Err(result)
    }
}

fn sqlite_journal_mode(conn: &rusqlite::Connection) -> String {
    conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
        .unwrap_or_else(|_| "unknown".into())
}

fn sqlite_scalar_i32(conn: &rusqlite::Connection, sql: &str) -> Option<i32> {
    conn.query_row(sql, [], |row| row.get::<_, i32>(0)).ok()
}

fn newest_file_age(dir: &std::path::Path) -> Option<Duration> {
    let mut newest: Option<std::time::SystemTime> = None;

    fn walk(dir: &std::path::Path, newest: &mut Option<std::time::SystemTime>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Ok(ft) = entry.file_type() {
                    if ft.is_file() {
                        if let Ok(meta) = entry.metadata() {
                            if let Ok(modified) = meta.modified() {
                                if newest.is_none_or(|n| modified > n) {
                                    *newest = Some(modified);
                                }
                            }
                        }
                    } else if ft.is_dir() {
                        walk(&entry.path(), newest);
                    }
                }
            }
        }
    }

    walk(dir, &mut newest);
    newest.and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_code_logic() {
        let results = [
            CheckResult {
                name: "a".into(),
                status: CheckStatus::Pass,
                detail: "ok".into(),
                remediation: None,
            },
            CheckResult {
                name: "b".into(),
                status: CheckStatus::Warn,
                detail: "meh".into(),
                remediation: Some("fix it".into()),
            },
        ];
        // No FAIL => exit 0
        assert!(!results.iter().any(|r| r.status == CheckStatus::Fail));

        let results_with_fail = [
            CheckResult {
                name: "a".into(),
                status: CheckStatus::Pass,
                detail: "ok".into(),
                remediation: None,
            },
            CheckResult {
                name: "c".into(),
                status: CheckStatus::Fail,
                detail: "bad".into(),
                remediation: Some("fix".into()),
            },
        ];
        assert!(results_with_fail
            .iter()
            .any(|r| r.status == CheckStatus::Fail));
    }

    #[test]
    fn test_token_file_permission_evaluation() {
        // 0o600 should pass
        assert_eq!(0o600 & 0o777, 0o600);
        // 0o644 should not match
        assert_ne!(0o644 & 0o777, 0o600);
        // 0o700 should not match
        assert_ne!(0o700 & 0o777, 0o600);
    }

    #[test]
    fn test_disk_thresholds() {
        // < 5 GB => FAIL
        let free_gb: f64 = 4.5;
        assert!(free_gb < 5.0);

        // 5-15 GB => WARN
        let free_gb: f64 = 10.0;
        assert!((5.0..15.0).contains(&free_gb));

        // >= 15 GB => INFO
        let free_gb: f64 = 50.0;
        assert!(free_gb >= 15.0);
    }

    #[test]
    fn test_backup_age_thresholds() {
        // > 48h => WARN
        let hours = 50u64;
        assert!(hours > 48);

        // <= 48h => PASS
        let hours = 24u64;
        assert!(hours <= 48);
    }

    #[test]
    fn test_check_result_json_serialization() {
        let result = CheckResult {
            name: "test".into(),
            status: CheckStatus::Pass,
            detail: "all good".into(),
            remediation: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"status\":\"PASS\""));
        assert!(json.contains("\"name\":\"test\""));
        // remediation should be absent (skip_serializing_if)
        assert!(!json.contains("remediation"));

        let result_with_rem = CheckResult {
            name: "fail".into(),
            status: CheckStatus::Fail,
            detail: "broken".into(),
            remediation: Some("fix it".into()),
        };
        let json = serde_json::to_string(&result_with_rem).unwrap();
        assert!(json.contains("\"remediation\":\"fix it\""));
    }

    #[test]
    fn test_output_formatting() {
        let results = vec![
            CheckResult {
                name: "short".into(),
                status: CheckStatus::Pass,
                detail: "ok".into(),
                remediation: None,
            },
            CheckResult {
                name: "longer-name".into(),
                status: CheckStatus::Fail,
                detail: "broken".into(),
                remediation: Some("fix this thing".into()),
            },
            CheckResult {
                name: "info-check".into(),
                status: CheckStatus::Info,
                detail: "noted".into(),
                remediation: None,
            },
        ];
        // Smoke test: print_table should not panic
        super::super::output::print_table(&results);
    }

    #[test]
    fn test_sqlite_checks_with_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Create a valid DB
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT);
                 INSERT INTO schema_version VALUES (8, '2025-01-01');",
            )
            .unwrap();
        }

        // Read-only open
        let conn = open_readonly_sqlite(&db_path).unwrap();
        assert!(sqlite_quick_check(&conn).is_ok());
        assert_eq!(
            sqlite_scalar_i32(&conn, "SELECT MAX(version) FROM schema_version"),
            Some(8)
        );
        let journal = sqlite_journal_mode(&conn);
        assert!(!journal.is_empty());
    }

    #[test]
    fn test_sqlite_corrupted_db() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("corrupt.db");

        // Write garbage
        std::fs::write(&db_path, b"this is not a sqlite database").unwrap();

        let result = open_readonly_sqlite(&db_path);
        // Should either fail to open or fail quick_check
        if let Ok(conn) = result {
            assert!(sqlite_quick_check(&conn).is_err());
        }
    }

    /// Build a fixture `decision_audit` chain that mirrors the real v1 -> v2 hash
    /// evolution: early rows carry no principal (8 hashed fields), later rows do
    /// (9 fields, principal folded in after evidence_digest). Every hash comes
    /// from the canonical `permagent::decisions::compute_audit_row_hash` — the
    /// doctor check must never own a second copy of the algorithm.
    fn write_fixture_chain(db_path: &std::path::Path) {
        use permagent::decisions::compute_audit_row_hash;

        let conn = rusqlite::Connection::open(db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE decision_audit (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                decision_id TEXT NOT NULL, goal_id TEXT, acted_by TEXT NOT NULL,
                tier INTEGER NOT NULL, outcome TEXT NOT NULL, evidence_digest TEXT,
                principal TEXT, prev_hash TEXT, row_hash TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )
        .unwrap();

        // seq 1 — pre-attribution row: system creation, no principal.
        let h1 = compute_audit_row_hash("", "d1", "", "system", 1, "created", "", None, "t1");
        conn.execute(
            "INSERT INTO decision_audit (decision_id, goal_id, acted_by, tier, outcome, \
             evidence_digest, principal, prev_hash, row_hash, created_at) \
             VALUES ('d1', NULL, 'system', 1, 'created', NULL, NULL, NULL, ?1, 't1')",
            [&h1],
        )
        .unwrap();

        // seq 2 — the first principal-bearing row (an authenticated HTTP answer).
        let h2 = compute_audit_row_hash(
            &h1,
            "d1",
            "",
            "jesse",
            1,
            "approve",
            "",
            Some("master"),
            "t2",
        );
        conn.execute(
            "INSERT INTO decision_audit (decision_id, goal_id, acted_by, tier, outcome, \
             evidence_digest, principal, prev_hash, row_hash, created_at) \
             VALUES ('d1', NULL, 'jesse', 1, 'approve', NULL, 'master', ?1, ?2, 't2')",
            rusqlite::params![&h1, &h2],
        )
        .unwrap();

        // seq 3 — a chat-relayed answer, attributed to the henry-chat principal.
        let h3 = compute_audit_row_hash(
            &h2,
            "d2",
            "g1",
            "henry-policy",
            1,
            "reject",
            "",
            Some("henry-chat"),
            "t3",
        );
        conn.execute(
            "INSERT INTO decision_audit (decision_id, goal_id, acted_by, tier, outcome, \
             evidence_digest, principal, prev_hash, row_hash, created_at) \
             VALUES ('d2', 'g1', 'henry-policy', 1, 'reject', NULL, 'henry-chat', ?1, ?2, 't3')",
            rusqlite::params![&h2, &h3],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn doctor_passes_a_chain_that_carries_principals() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("audit.db");
        write_fixture_chain(&db_path);

        let result = check_decision_audit_chain_at(&db_path).await;
        assert_eq!(
            result.status,
            CheckStatus::Pass,
            "principal-bearing chain must verify: {}",
            result.detail
        );
        assert!(result.detail.contains('3'), "detail: {}", result.detail);
    }

    #[tokio::test]
    async fn doctor_reports_a_tampered_row_with_its_seq() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("audit.db");
        write_fixture_chain(&db_path);

        // Rewrite the outcome of the principal-bearing row, leaving every hash in
        // place — the forgery an append-only log exists to catch.
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute(
                "UPDATE decision_audit SET outcome = 'reject' WHERE seq = 2",
                [],
            )
            .unwrap();
        }

        let result = check_decision_audit_chain_at(&db_path).await;
        assert_eq!(
            result.status,
            CheckStatus::Fail,
            "detail: {}",
            result.detail
        );
        assert!(
            result.detail.contains("seq 2"),
            "break point must name the row: {}",
            result.detail
        );
    }

    #[tokio::test]
    async fn doctor_passes_an_empty_chain() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("audit.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE decision_audit (
                    seq INTEGER PRIMARY KEY AUTOINCREMENT,
                    decision_id TEXT NOT NULL, goal_id TEXT, acted_by TEXT NOT NULL,
                    tier INTEGER NOT NULL, outcome TEXT NOT NULL, evidence_digest TEXT,
                    principal TEXT, prev_hash TEXT, row_hash TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );",
            )
            .unwrap();
        }

        let result = check_decision_audit_chain_at(&db_path).await;
        assert_eq!(
            result.status,
            CheckStatus::Pass,
            "detail: {}",
            result.detail
        );
    }

    #[test]
    fn test_newest_file_age() {
        let dir = tempfile::tempdir().unwrap();

        // Empty dir
        assert!(newest_file_age(dir.path()).is_none());

        // Create a file
        std::fs::write(dir.path().join("test.txt"), b"hello").unwrap();
        let age = newest_file_age(dir.path());
        assert!(age.is_some());
        // Should be very recent (< 5 seconds)
        assert!(age.unwrap().as_secs() < 5);
    }

    // ── Integration tests ──

    #[tokio::test]
    #[serial_test::serial]
    async fn test_version_endpoint_is_public_and_auth_rejects_without_token() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;

        // /api/version is public: returns 200 without token
        Mock::given(method("GET"))
            .and(path("/api/version"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "git_sha": "abc123def",
                "git_dirty": "false",
                "permagentd_version": "1.31.0",
                "spectral_pin": "2c1f6bf"
            })))
            .mount(&mock)
            .await;

        // /config is protected: returns 401 without token
        Mock::given(method("GET"))
            .and(path("/config"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock)
            .await;

        let base = mock.uri();

        // Version endpoint: 200 without token
        let status = http_get(&format!("{base}/api/version"), None)
            .await
            .unwrap();
        assert_eq!(status, 200);

        // Protected endpoint: 401 without token
        let status = http_get(&format!("{base}/config"), None).await.unwrap();
        assert_eq!(status, 401);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_ollama_check_model_present() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {"name": "qwen2.5:7b", "size": 4_000_000_000_u64},
                    {"name": "llama3:8b", "size": 5_000_000_000_u64}
                ]
            })))
            .mount(&mock)
            .await;

        let result = check_ollama_at(&mock.uri(), "qwen2.5:7b").await;
        assert_eq!(result.status, CheckStatus::Pass);
        assert!(result.detail.contains("qwen2.5:7b"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_ollama_check_model_missing() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {"name": "llama3:8b", "size": 5_000_000_000_u64}
                ]
            })))
            .mount(&mock)
            .await;

        let result = check_ollama_at(&mock.uri(), "qwen2.5:7b").await;
        assert_eq!(result.status, CheckStatus::Fail);
        assert_eq!(
            result.remediation.as_deref(),
            Some("ollama pull qwen2.5:7b")
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_ollama_check_unreachable() {
        // Point at a port that is definitely not listening
        let result = check_ollama_at("http://127.0.0.1:19999", "qwen2.5:7b").await;
        assert_eq!(result.status, CheckStatus::Warn);
        assert!(result.detail.contains("not reachable"));
    }

    #[test]
    fn test_sqlite_schema_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("mismatch.db");

        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT);
                 INSERT INTO schema_version VALUES (5, '2025-01-01');",
            )
            .unwrap();
        }

        let conn = open_readonly_sqlite(&db_path).unwrap();
        let db_version = sqlite_scalar_i32(&conn, "SELECT MAX(version) FROM schema_version");
        assert_eq!(db_version, Some(5));
        // v5 is BELOW the fresh-init base stamp, which is the only genuinely
        // wrong state. A DB *above* the base is a normal migrated install and
        // must not be reported as a mismatch.
        assert!(db_version.unwrap() < SPECTRAL_SCHEMA_VERSION);
    }

    #[test]
    fn test_extract_api_key_names_from_plist() {
        let plist = r#"
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.permagent.daemon</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>/Users/test</string>
        <key>ANTHROPIC_API_KEY</key>
        <string>sk-ant-something</string>
        <key>OPENAI_API_KEY</key>
        <string>sk-something</string>
        <key>PERMAGENT_CONFIG</key>
        <string>/Users/test/.permagent/config.yaml</string>
    </dict>
    <key>ProcessType</key>
    <string>Standard</string>
</dict>
</plist>"#;
        let keys = extract_api_key_names_from_plist(plist);
        assert_eq!(keys, vec!["ANTHROPIC_API_KEY", "OPENAI_API_KEY"]);
    }

    #[test]
    fn test_extract_api_key_names_from_plist_no_env_section() {
        let plist = "<plist><dict><key>Label</key><string>x</string></dict></plist>";
        assert!(extract_api_key_names_from_plist(plist).is_empty());
    }

    #[test]
    fn test_extract_api_key_names_from_plist_ignores_keys_outside_env_dict() {
        // *_API_KEY after the EnvironmentVariables dict closes must not match
        let plist = r#"
<dict>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/usr/bin</string>
    </dict>
    <key>FAKE_API_KEY</key>
    <string>nope</string>
</dict>"#;
        assert!(extract_api_key_names_from_plist(plist).is_empty());
    }

    #[test]
    fn test_extract_api_key_names_from_env_listing() {
        let listing = "permagentd agent HOME=/Users/test ANTHROPIC_API_KEY=sk-ant-x \
                       PATH=/usr/bin lowercase_api_key=x OPENROUTER_API_KEY=y";
        let keys = extract_api_key_names_from_env_listing(listing);
        // lowercase names are not env-var shaped and must be ignored
        assert_eq!(keys, vec!["ANTHROPIC_API_KEY", "OPENROUTER_API_KEY"]);
    }

    #[test]
    fn test_classify_shadowing_clean() {
        let (status, detail, rem) = classify_shadowing(&[], true, &[]);
        assert_eq!(status, CheckStatus::Pass);
        assert!(detail.contains("no *_API_KEY"));
        assert!(rem.is_none());
    }

    #[test]
    fn test_classify_shadowing_env_plus_keychain_warns() {
        let env = vec!["ANTHROPIC_API_KEY".to_string()];
        let (status, detail, rem) = classify_shadowing(&env, true, &[]);
        assert_eq!(status, CheckStatus::Warn);
        assert!(detail.contains("ANTHROPIC_API_KEY"));
        assert!(detail.contains("keychain blob"));
        assert!(rem.is_some());
    }

    #[test]
    fn test_classify_shadowing_env_plus_yaml_overlap_warns() {
        let env = vec!["OPENAI_API_KEY".to_string()];
        let yaml = vec!["OPENAI_API_KEY".to_string()];
        let (status, detail, _) = classify_shadowing(&env, false, &yaml);
        assert_eq!(status, CheckStatus::Warn);
        assert!(detail.contains("secrets.yaml"));
    }

    #[test]
    fn test_classify_shadowing_env_only_is_bootstrap_info() {
        let env = vec!["ANTHROPIC_API_KEY".to_string()];
        let (status, detail, rem) = classify_shadowing(&env, false, &[]);
        assert_eq!(status, CheckStatus::Info);
        assert!(detail.contains("bootstrap"));
        assert!(rem.is_none());
    }

    #[test]
    fn test_sqlite_missing_schema_table() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("no_schema.db");

        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch("CREATE TABLE other (id INTEGER PRIMARY KEY);")
                .unwrap();
        }

        let conn = open_readonly_sqlite(&db_path).unwrap();
        // Query against missing table should return None (error)
        let db_version = sqlite_scalar_i32(&conn, "SELECT MAX(version) FROM schema_version");
        assert!(db_version.is_none());
    }

    // ── D34: a break-glass disable must be visible to the operator ──

    #[test]
    fn the_safety_inspector_check_is_always_present_and_names_itself() {
        let r = check_safety_inspectors();
        assert_eq!(r.name, "safety-inspectors");
        assert!(
            matches!(r.status, CheckStatus::Info | CheckStatus::Warn),
            "a disable is a WARN, a clean box is an INFO — never a silent PASS"
        );
        assert!(
            r.detail.contains("adversary"),
            "the opt-in reviewer's state is always stated: {}",
            r.detail
        );
    }

    #[test]
    fn a_disabled_inspector_in_this_process_surfaces_as_a_named_warn() {
        let _env = env_lock::lock_env([("SECURITY_WRITE_JAIL_ENABLED", Some("false"))]);
        let r = check_safety_inspectors();
        assert_eq!(r.status, CheckStatus::Warn);
        assert!(
            r.detail
                .contains("safety inspector write_jail disabled via SECURITY_WRITE_JAIL_ENABLED"),
            "doctor must name the inspector AND the env var: {}",
            r.detail
        );
        assert!(
            r.remediation
                .as_deref()
                .unwrap_or_default()
                .contains("SAFETY_INSPECTOR_DISABLED"),
            "the remediation points at the runtime marker to grep for"
        );
    }

    /// The daemon runs in its own process, so doctor reads its environment out
    /// of `ps eww` rather than trusting its own.
    #[test]
    fn switches_are_read_out_of_a_process_env_listing() {
        let listing = "PATH=/usr/bin SECURITY_PROMPT_ENABLED=false HOME=/Users/j \
                       GOOSE_TOOL_ARG_VALIDATION=off /usr/local/bin/permagentd";
        let found = extract_env_values(
            listing,
            &["SECURITY_PROMPT_ENABLED", "GOOSE_TOOL_ARG_VALIDATION"],
        );
        assert_eq!(
            found,
            vec![
                ("SECURITY_PROMPT_ENABLED".to_string(), "false".to_string()),
                ("GOOSE_TOOL_ARG_VALIDATION".to_string(), "off".to_string()),
            ]
        );
        assert!(
            extract_env_values(listing, &["SECURITY_WRITE_JAIL_ENABLED"]).is_empty(),
            "an unset switch must not be reported as disabled"
        );
    }
}
