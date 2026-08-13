//! Picker — the stock-scanning service the user already runs, as a seam.
//!
//! `~/dev/Picker/pre_surge_scanner` is a Flask service under launchd that owns
//! the pre-surge ranking algorithm, its backtests, its leaderboard and its
//! trade history. It is not ours to reimplement, and we do not spawn its
//! Python: it is a long-lived service with an HTTP API, so this module is an
//! HTTP client and nothing more.
//!
//! That choice removes a whole failure class before it exists. Driving a heavy
//! scan as a child process would mean owning its lifetime — process groups,
//! timeouts, orphan reaping, a scan surviving a daemon restart. Asking a
//! service to scan means the scan's lifetime is the service's problem, which
//! is where it already lived.
//!
//! ## Honesty
//!
//! The scanner is frequently NOT running. Every call here distinguishes
//! "unreachable" from "reachable and has nothing", because a stale pick
//! rendered as today's pick is worse than an empty surface — it is a
//! recommendation about a market that has since moved.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Config key overriding where the scanner lives.
pub const PICKER_URL_KEY: &str = "picker_url";
/// Explicit override for the Picker checkout location, for a layout the
/// conventional search below does not cover.
pub const PICKER_ROOT_KEY: &str = "picker_root";

/// Loopback by default. The service has no authentication and its API can
/// modify the trade history and start expensive scans, so it should not be
/// reachable off this machine; see the bind comment in `csv_web_server.py`.
const DEFAULT_BASE: &str = "http://127.0.0.1:8080";

/// Short: a scanner that is down should be reported as down promptly, not
/// waited on. The scan-triggering call uses its own, longer bound.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// Where the user's Picker checkout lives, for bringing the service up.
/// Locate the Picker checkout.
///
/// `~/dev/Picker` was hardcoded, which is only true on a machine whose repos
/// live directly under $HOME. On 2026-08-13 this made the Financier report
/// "nothing to start" on a machine where the checkout was sitting in
/// `~/Documents/dev/Picker/pre_surge_scanner` — the third place in this
/// codebase where a `~/dev` assumption survived a move between Macs (the
/// storage scanner and the project root_paths were the others).
///
/// `picker_root` config key wins, so a checkout anywhere can be named
/// explicitly rather than requiring a conventional layout.
fn picker_root() -> Option<std::path::PathBuf> {
    if let Ok(configured) = crate::config::Config::global().get_param::<String>(PICKER_ROOT_KEY) {
        let p = std::path::PathBuf::from(shellexpand::tilde(&configured).into_owned());
        if p.is_dir() {
            return Some(p);
        }
    }

    let home = dirs::home_dir()?;
    [
        "dev",
        "Documents/dev",
        "code",
        "Documents/code",
        "projects",
        "src",
    ]
    .iter()
    .map(|base| home.join(base).join("Picker/pre_surge_scanner"))
    .find(|p| p.is_dir())
}

pub fn base_url() -> String {
    crate::config::Config::global()
        .get_param::<String>(PICKER_URL_KEY)
        .ok()
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())
}

/// What the scanner is doing right now. `reachable: false` is a first-class
/// answer, never an error dressed up as empty data.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct PickerStatus {
    pub reachable: bool,
    pub base_url: String,
    /// A scan is in flight — picks will change shortly.
    pub scan_in_progress: bool,
    /// Date of the scan the current results came from, if any.
    pub scan_date: Option<String>,
    /// How many ranked results the last scan produced.
    pub results: Option<u64>,
    /// Why it is unreachable, when it is.
    pub detail: Option<String>,
}

pub async fn status() -> PickerStatus {
    let base = base_url();
    let mut out = PickerStatus {
        base_url: base.clone(),
        ..Default::default()
    };
    let Ok(c) = client(PROBE_TIMEOUT) else {
        out.detail = Some("could not build an HTTP client".into());
        return out;
    };
    match c.get(format!("{base}/api/status")).send().await {
        Ok(resp) if resp.status().is_success() => {
            out.reachable = true;
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                out.scan_in_progress = v
                    .get("scan_in_progress")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false);
                out.scan_date = v
                    .get("scan_date")
                    .and_then(|s| s.as_str())
                    .map(str::to_string);
                out.results = v
                    .get("total_results")
                    .or_else(|| v.get("results"))
                    .and_then(|n| n.as_u64());
            }
        }
        Ok(resp) => {
            out.detail = Some(format!("scanner answered {}", resp.status()));
        }
        Err(e) => {
            out.detail = Some(if e.is_connect() || e.is_timeout() {
                "the scanner is not running".to_string()
            } else {
                e.to_string()
            });
        }
    }
    out
}

/// The ranked picks the last scan produced. `Err` means we could not ask;
/// `Ok(empty)` means the scanner answered and had nothing. The caller must not
/// collapse those two.
pub async fn top_picks() -> Result<Vec<serde_json::Value>, String> {
    let base = base_url();
    let resp = client(CALL_TIMEOUT)?
        .get(format!("{base}/api/top-picks"))
        .send()
        .await
        .map_err(|e| unreachable_msg(&e))?;
    if !resp.status().is_success() {
        return Err(format!("scanner answered {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    // The route has answered as a bare array and as `{picks: [...]}` across
    // revisions; accept either rather than silently reading zero picks.
    let picks = v
        .get("picks")
        .or_else(|| v.get("top_picks"))
        .and_then(|p| p.as_array())
        .or_else(|| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(picks)
}

/// Ask the scanner to run. Returns as soon as the scan is ACCEPTED — a full
/// scan takes many minutes, so waiting for it here would block a tool call
/// past any sensible bound. Poll [`status`] for progress.
pub async fn start_scan() -> Result<String, String> {
    let base = base_url();
    let resp = client(CALL_TIMEOUT)?
        .post(format!("{base}/api/scan"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| unreachable_msg(&e))?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    if !status.is_success() {
        return Err(body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("the scanner refused the scan")
            .to_string());
    }
    Ok(body
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("scan started")
        .to_string())
}

/// One trade for the user's history. Mirrors the scanner's own required
/// fields — a partial trade is rejected there, so it is rejected here first
/// with a message that says which field is missing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeEntry {
    /// ISO date, `YYYY-MM-DD`.
    pub entry_date: String,
    pub ticker: String,
    pub company_name: String,
    pub entry_price: f64,
    pub shares: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Record a trade the user actually made.
///
/// This WRITES to the user's trade history, which is the record their
/// performance is measured against — a wrong entry is not a display bug, it
/// silently poisons every backtest and hit-rate number computed afterwards.
/// So: no defaults invented here, and the caller states every required field.
pub async fn record_trade(trade: &TradeEntry) -> Result<serde_json::Value, String> {
    if trade.ticker.trim().is_empty() {
        return Err("ticker is required".into());
    }
    if trade.shares <= 0 {
        return Err("shares must be a positive number".into());
    }
    if !(trade.entry_price.is_finite() && trade.entry_price > 0.0) {
        return Err("entry_price must be a positive number".into());
    }
    if !is_iso_date(&trade.entry_date) {
        return Err("entry_date must be an ISO date, YYYY-MM-DD".into());
    }
    if let Some(d) = trade.exit_date.as_deref() {
        if !is_iso_date(d) {
            return Err("exit_date must be an ISO date, YYYY-MM-DD".into());
        }
    }

    let base = base_url();
    let resp = client(CALL_TIMEOUT)?
        .post(format!("{base}/api/trades"))
        .json(trade)
        .send()
        .await
        .map_err(|e| unreachable_msg(&e))?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    if !status.is_success() {
        return Err(body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("the scanner rejected the trade")
            .to_string());
    }
    Ok(body)
}

/// The trades already recorded.
pub async fn trades() -> Result<Vec<serde_json::Value>, String> {
    let base = base_url();
    let resp = client(CALL_TIMEOUT)?
        .get(format!("{base}/api/trades"))
        .send()
        .await
        .map_err(|e| unreachable_msg(&e))?;
    if !resp.status().is_success() {
        return Err(format!("scanner answered {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(v.get("trades")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default())
}

/// Bring the scanner up through launchd — the mechanism it was already built
/// to run under (`com.picker.stockscanner.plist`, `KeepAlive`), so it survives
/// a restart of this daemon rather than dying with it. Spawning the server as
/// our own child would tie its life to ours, which is exactly wrong for a
/// service the user runs independently.
pub async fn ensure_running() -> Result<String, String> {
    if status().await.reachable {
        return Ok("the scanner was already running".into());
    }
    if !cfg!(target_os = "macos") {
        return Err("starting the scanner is wired for launchd (macOS) only".into());
    }
    let root = picker_root().ok_or_else(|| {
        "no Picker checkout at ~/dev/Picker/pre_surge_scanner — nothing to start".to_string()
    })?;
    let plist_src = root.join("com.picker.stockscanner.plist");
    if !plist_src.is_file() {
        return Err(format!(
            "no launchd job at {} — start the scanner manually",
            plist_src.display()
        ));
    }
    let home = dirs::home_dir().ok_or("no home directory")?;
    let agents = home.join("Library/LaunchAgents");
    tokio::fs::create_dir_all(&agents)
        .await
        .map_err(|e| e.to_string())?;
    let plist_dst = agents.join("com.picker.stockscanner.plist");
    tokio::fs::copy(&plist_src, &plist_dst)
        .await
        .map_err(|e| format!("could not install the launch agent: {e}"))?;

    // `id -u` rather than a libc call: this path is macOS-only and shelling
    // out keeps the module free of a platform-gated dependency for one number.
    let uid = run("id", &["-u"]).await?;
    let target = format!("gui/{uid}");
    // Bootstrap, then kickstart: bootstrap fails if it is already loaded, and
    // "already loaded but not running" is a state we still want to fix.
    let _ = run(
        "launchctl",
        &["bootstrap", &target, &plist_dst.to_string_lossy()],
    )
    .await;
    run(
        "launchctl",
        &[
            "kickstart",
            "-k",
            &format!("{target}/com.picker.stockscanner"),
        ],
    )
    .await?;

    Ok(format!(
        "asked launchd to start the scanner ({}). It takes a few seconds to bind {}.",
        plist_dst.display(),
        base_url()
    ))
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

fn unreachable_msg(e: &reqwest::Error) -> String {
    if e.is_connect() || e.is_timeout() {
        format!(
            "the scanner is not running at {} — start it first",
            base_url()
        )
    } else {
        e.to_string()
    }
}

/// `YYYY-MM-DD`, checked structurally. Not a full calendar validation — the
/// point is to reject "yesterday", "08/07/26" and an empty string before they
/// reach a trade row that every later performance number is computed from.
fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates_only() {
        assert!(is_iso_date("2026-08-07"));
        assert!(!is_iso_date("08/07/2026"));
        assert!(!is_iso_date("yesterday"));
        assert!(!is_iso_date(""));
        assert!(!is_iso_date("2026-8-7"));
        assert!(!is_iso_date("2026-08-07T00:00:00Z"));
    }

    fn valid_trade() -> TradeEntry {
        TradeEntry {
            entry_date: "2026-08-07".into(),
            ticker: "ENB".into(),
            company_name: "Enbridge".into(),
            entry_price: 42.5,
            shares: 100,
            exit_date: None,
            exit_price: None,
            notes: None,
        }
    }

    /// Validation runs BEFORE any request, so a malformed trade fails with a
    /// message naming the field rather than as a connection error — and it
    /// fails the same way whether or not the scanner happens to be up.
    #[tokio::test]
    async fn a_malformed_trade_is_rejected_by_field_not_by_the_network() {
        let cases: Vec<(TradeEntry, &str)> = vec![
            (
                TradeEntry {
                    ticker: "  ".into(),
                    ..valid_trade()
                },
                "ticker",
            ),
            (
                TradeEntry {
                    shares: 0,
                    ..valid_trade()
                },
                "shares",
            ),
            (
                TradeEntry {
                    entry_price: 0.0,
                    ..valid_trade()
                },
                "entry_price",
            ),
            (
                TradeEntry {
                    entry_price: f64::NAN,
                    ..valid_trade()
                },
                "entry_price",
            ),
            (
                TradeEntry {
                    entry_date: "yesterday".into(),
                    ..valid_trade()
                },
                "entry_date",
            ),
            (
                TradeEntry {
                    exit_date: Some("soon".into()),
                    ..valid_trade()
                },
                "exit_date",
            ),
        ];
        for (trade, field) in cases {
            let err = record_trade(&trade).await.expect_err("must be rejected");
            assert!(
                err.contains(field),
                "error should name the offending field, got: {err}"
            );
        }
    }

    #[test]
    fn the_default_base_is_loopback() {
        // The scanner has no auth and can modify trades; it must not be
        // addressed off-machine by default.
        assert!(DEFAULT_BASE.starts_with("http://127.0.0.1"));
    }
}
