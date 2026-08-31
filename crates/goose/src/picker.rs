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
/// Off until the user turns Picker on from the Finance tab. The ranked list
/// and scanner card stay hidden without this — other people should not inherit
/// someone else's stock desk.
pub const PICKER_ENABLED_KEY: &str = "picker_enabled";
/// User-supplied ticker universe. Comma, space, or newline separated.
/// When set, a scan ranks these names rather than a checkout-specific list.
pub const PICKER_UNIVERSE_KEY: &str = "picker_universe";
/// Cap so a pasted dump cannot fan out into hundreds of Yahoo + loop calls.
pub const MAX_UNIVERSE: usize = 80;

/// True only when the user has opted in. Missing key is off.
pub fn is_enabled() -> bool {
    crate::config::Config::global()
        .get_param::<bool>(PICKER_ENABLED_KEY)
        .unwrap_or(false)
}

/// Tickers the user listed. Empty means "use the scanner's own universe if it
/// is running" — never a silent default list of someone else's ideas.
pub fn universe() -> Vec<String> {
    crate::config::Config::global()
        .get_param::<String>(PICKER_UNIVERSE_KEY)
        .ok()
        .map(|s| parse_universe(&s))
        .unwrap_or_default()
}

/// Split a pasted universe into unique uppercase tickers.
pub fn parse_universe(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in raw.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let t = token.trim().trim_start_matches('$').to_uppercase();
        if !is_ticker(&t) {
            continue;
        }
        if out.iter().any(|s| s == &t) {
            continue;
        }
        out.push(t);
        if out.len() >= MAX_UNIVERSE {
            break;
        }
    }
    out
}

fn is_ticker(s: &str) -> bool {
    let len = s.chars().count();
    if len == 0 || len > 12 {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && s.chars().any(|c| c.is_ascii_alphabetic())
}

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

    // Shared resolver: onboarding asks where this user keeps code, and every
    // path-guessing feature must read the same answer rather than inventing its
    // own (see config::dev_roots for the four that did).
    crate::config::dev_roots::dev_roots()
        .into_iter()
        .map(|root| root.join("Picker/pre_surge_scanner"))
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
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                if looks_like_picker(&v) {
                    out.reachable = true;
                    out.scan_in_progress = v
                        .get("scan_in_progress")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);
                    out.scan_date = v
                        .get("scan_date")
                        .or_else(|| v.get("last_scan"))
                        .and_then(|s| s.as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    out.results = v
                        .get("total_results")
                        .or_else(|| v.get("results"))
                        .and_then(|n| n.as_u64());
                } else {
                    out.detail = Some(format!(
                        "{base} answered, but it is not the stock scanner. \
                         The Librarian's nightly llama-server split moved off \
                         this port to :8081 on 2026-08-25 — if something is \
                         still serving llama-server here, it is an old \
                         qwen38-split.sh. Stop that process or set picker_url \
                         to the scanner."
                    ));
                }
            } else {
                out.detail = Some(format!(
                    "{base} answered, but the body is not scanner JSON — \
                     something else is bound to that port (the Librarian \
                     split serves :8081 now, so a stale one here is the \
                     usual culprit)"
                ));
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

/// True for the Flask scanner's `/api/status` body. llama-server on the same
/// port speaks a different JSON (`{"status":"ok"}` on `/health`) and must not
/// be treated as a live scanner — that is how a down Picker looked "up".
pub fn looks_like_picker(v: &serde_json::Value) -> bool {
    if v.get("scan_in_progress")
        .and_then(|b| b.as_bool())
        .is_some()
    {
        return true;
    }
    matches!(
        v.get("status").and_then(|s| s.as_str()),
        Some("running" | "idle" | "complete" | "ready")
    )
}

/// Pull a picks array out of a scanner body. Live Picker serves `/api/results`
/// (`results: [...]`); an older revision used `/api/top-picks`. Accept either
/// rather than silently reading zero picks.
pub fn picks_from_body(v: &serde_json::Value) -> Vec<serde_json::Value> {
    v.get("picks")
        .or_else(|| v.get("top_picks"))
        .or_else(|| v.get("results"))
        .and_then(|p| p.as_array())
        .or_else(|| v.as_array())
        .cloned()
        .unwrap_or_default()
}

/// The ranked picks the last scan produced. `Err` means we could not ask;
/// `Ok(empty)` means the scanner answered and had nothing. The caller must not
/// collapse those two.
///
/// Tries `/api/top-picks` then `/api/results` — the live scanner only has the
/// second route.
pub async fn top_picks() -> Result<Vec<serde_json::Value>, String> {
    let base = base_url();
    let c = client(CALL_TIMEOUT)?;
    for path in ["/api/top-picks", "/api/results"] {
        let resp = match c.get(format!("{base}{path}")).send().await {
            Ok(r) => r,
            Err(e) => return Err(unreachable_msg(&e)),
        };
        if !resp.status().is_success() {
            if path == "/api/top-picks" {
                continue;
            }
            return Err(format!("scanner answered {}", resp.status()));
        }
        let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let picks = picks_from_body(&v);
        if !picks.is_empty() || path == "/api/results" {
            return Ok(picks);
        }
    }
    Ok(Vec::new())
}

/// Tickers the user added on the Finance tab. Overlay on the scanner's own
/// universe — never a replacement. Empty is the usual case.
pub fn extras() -> Vec<String> {
    universe()
}

/// How many names the Picker checkout is ranking, when we can count them.
/// Reads `data/ticker_universe_cache.json` next to the scanner; a miss is
/// `None`, not zero (zero would look like an empty universe).
pub fn checkout_ticker_count() -> Option<(u64, String)> {
    let root = picker_root()?;
    let candidates = [
        root.join("data/ticker_universe_cache.json"),
        root.join("../data/ticker_universe_cache.json"),
    ];
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let count = text.matches("\"is_active\": true").count() as u64;
        if count > 0 {
            return Some((count, path.display().to_string()));
        }
    }
    None
}

/// Ask the scanner to run. Returns as soon as the scan is ACCEPTED — a full
/// scan takes many minutes, so waiting for it here would block a tool call
/// past any sensible bound. Poll [`status`] for progress.
///
/// The scanner keeps its own checkout universe. Finance-tab extras are an
/// overlay ranked on the tab, not a replacement list sent here.
pub async fn start_scan() -> Result<String, String> {
    start_scan_with(&[]).await
}

pub async fn start_scan_with(tickers: &[String]) -> Result<String, String> {
    let base = base_url();
    let body = if tickers.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::json!({ "tickers": tickers })
    };
    let resp = client(CALL_TIMEOUT)?
        .post(format!("{base}/api/scan"))
        .json(&body)
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

fn validate_trade(trade: &TradeEntry) -> Result<(), String> {
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
    if let Some(px) = trade.exit_price {
        if !(px.is_finite() && px > 0.0) {
            return Err("exit_price must be a positive number".into());
        }
    }
    Ok(())
}

fn trade_body(id: Option<&str>, trade: &TradeEntry) -> serde_json::Value {
    let mut v = serde_json::to_value(trade).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(id) = id.filter(|s| !s.is_empty()) {
        if let Some(obj) = v.as_object_mut() {
            obj.insert("id".into(), serde_json::Value::String(id.to_string()));
        }
    }
    v
}

/// Record a trade the user actually made.
///
/// This WRITES to the user's trade history, which is the record their
/// performance is measured against — a wrong entry is not a display bug, it
/// silently poisons every backtest and hit-rate number computed afterwards.
/// So: no defaults invented here, and the caller states every required field.
pub async fn record_trade(trade: &TradeEntry) -> Result<serde_json::Value, String> {
    validate_trade(trade)?;

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

/// Correct an already-recorded trade (shares, prices, notes, dates). PUT then
/// PATCH then POST `…/update` — Picker revisions have used all three. Does
/// not place an order.
pub async fn update_trade(id: &str, trade: &TradeEntry) -> Result<serde_json::Value, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("trade id is required".into());
    }
    validate_trade(trade)?;
    let body = trade_body(Some(id), trade);
    let encoded = urlencoding::encode(id);
    let attempts = [
        ("PUT", format!("/api/trades/{encoded}")),
        ("PATCH", format!("/api/trades/{encoded}")),
        ("POST", format!("/api/trades/{encoded}/update")),
    ];
    let mut last = "scanner has no update-trade route".to_string();
    for (method, path) in attempts {
        match json_call(method, &path, Some(&body)).await {
            Ok(v) => return Ok(v),
            Err(e) if is_missing_route(&e) => {
                last = e;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// Close an already-recorded trade in the scanner history. Same honesty as
/// [`record_trade`]: exit date and price come from the user, never inferred.
/// Prefers POST `…/close` and PATCH (partial) before PUT, so a replace-style
/// PUT does not wipe ticker/shares. `existing` is sent on PUT so a full
/// replace still keeps the lot. Does not place an order.
pub async fn close_trade(
    id: &str,
    exit_date: &str,
    exit_price: f64,
    existing: Option<&TradeEntry>,
) -> Result<serde_json::Value, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("trade id is required".into());
    }
    if !is_iso_date(exit_date) {
        return Err("exit_date must be an ISO date, YYYY-MM-DD".into());
    }
    if !(exit_price.is_finite() && exit_price > 0.0) {
        return Err("exit_price must be a positive number".into());
    }
    let partial = serde_json::json!({
        "id": id,
        "exit_date": exit_date,
        "exit_price": exit_price,
    });
    let encoded = urlencoding::encode(id);
    let close_path = format!("/api/trades/{encoded}/close");
    match json_call("POST", &close_path, Some(&partial)).await {
        Ok(v) => return Ok(v),
        Err(e) if is_missing_route(&e) => {}
        Err(e) => return Err(e),
    }
    match json_call("PATCH", &format!("/api/trades/{encoded}"), Some(&partial)).await {
        Ok(v) => return Ok(v),
        Err(e) if is_missing_route(&e) => {}
        Err(e) => return Err(e),
    }
    let put_body = if let Some(t) = existing {
        let mut full = t.clone();
        full.exit_date = Some(exit_date.to_string());
        full.exit_price = Some(exit_price);
        validate_trade(&full)?;
        trade_body(Some(id), &full)
    } else {
        partial
    };
    json_call("PUT", &format!("/api/trades/{encoded}"), Some(&put_body)).await
}

/// Remove a trade from scanner history. DELETE `/api/trades/<id>`, with a
/// POST-to-delete fallback some Flask revisions used.
pub async fn delete_trade(id: &str) -> Result<(), String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("trade id is required".into());
    }
    let encoded = urlencoding::encode(id);
    let attempts = [
        ("DELETE", format!("/api/trades/{encoded}")),
        ("POST", format!("/api/trades/{encoded}/delete")),
    ];
    let mut last = "scanner has no delete-trade route".to_string();
    for (method, path) in attempts {
        match json_call(method, &path, None).await {
            Ok(_) => return Ok(()),
            Err(e) if is_missing_route(&e) => {
                last = e;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

async fn json_call(
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let base = base_url();
    let url = format!("{base}{path}");
    let c = client(CALL_TIMEOUT)?;
    let mut req = match method {
        "PUT" => c.put(&url),
        "PATCH" => c.patch(&url),
        "POST" => c.post(&url),
        "DELETE" => c.delete(&url),
        other => return Err(format!("unsupported method {other}")),
    };
    if let Some(b) = body {
        req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| unreachable_msg(&e))?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.unwrap_or_default();
    if status.as_u16() == 404 || status.as_u16() == 405 {
        return Err(format!("scanner answered {status}"));
    }
    if !status.is_success() {
        return Err(v
            .get("error")
            .and_then(|e| e.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("scanner answered {status}")));
    }
    Ok(v)
}

fn is_missing_route(err: &str) -> bool {
    err.contains("404") || err.contains("405") || err.contains("not found")
}

/// One recorded trade, as the Finance tab displays it. Parsed defensively
/// from the scanner's JSON — field names have drifted across revisions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TradeRow {
    pub id: String,
    pub ticker: String,
    pub company_name: String,
    pub entry_date: String,
    pub entry_price: f64,
    pub shares: i64,
    pub exit_date: Option<String>,
    pub exit_price: Option<f64>,
    pub notes: Option<String>,
}

pub fn parse_trade_row(v: &serde_json::Value) -> Option<TradeRow> {
    let ticker = v
        .get("ticker")
        .or_else(|| v.get("symbol"))
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_uppercase();
    let entry_price = json_number(v, &["entry_price", "entryPrice", "price"])?;
    let shares = json_number(v, &["shares", "qty", "quantity"])? as i64;
    if shares == 0 {
        return None;
    }
    Some(TradeRow {
        id: v
            .get("id")
            .map(|id| match id {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                format!(
                    "{ticker}-{}",
                    v.get("entry_date").and_then(|s| s.as_str()).unwrap_or("?")
                )
            }),
        ticker,
        company_name: v
            .get("company_name")
            .or_else(|| v.get("companyName"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        entry_date: v
            .get("entry_date")
            .or_else(|| v.get("entryDate"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        entry_price,
        shares,
        exit_date: v
            .get("exit_date")
            .or_else(|| v.get("exitDate"))
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        exit_price: json_number(v, &["exit_price", "exitPrice"]),
        notes: v
            .get("notes")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    })
}

impl From<&TradeRow> for TradeEntry {
    fn from(t: &TradeRow) -> Self {
        Self {
            entry_date: t.entry_date.clone(),
            ticker: t.ticker.clone(),
            company_name: t.company_name.clone(),
            entry_price: t.entry_price,
            shares: t.shares,
            exit_date: t.exit_date.clone(),
            exit_price: t.exit_price,
            notes: t.notes.clone(),
        }
    }
}

fn json_number(v: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(n) = v.get(*k) {
            if let Some(f) = n.as_f64() {
                return Some(f);
            }
            if let Some(s) = n.as_str() {
                if let Some(f) = parse_money(s) {
                    return Some(f);
                }
            }
        }
    }
    None
}

/// "$12.34", "12.34%", "1,240.00" → f64. None if unreadable.
pub fn parse_money(raw: &str) -> Option<f64> {
    let s = raw
        .trim()
        .trim_start_matches('$')
        .trim_end_matches('%')
        .replace(',', "");
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("n/a") {
        return None;
    }
    s.parse().ok()
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
    let already = status().await;
    if already.reachable {
        return Ok("the scanner was already running".into());
    }
    if already.detail.as_deref().is_some_and(|d| {
        d.contains("not the stock scanner") || d.contains("something else is bound")
    }) {
        return Err(already.detail.unwrap_or_else(|| {
            format!(
                "{} is occupied by something that is not the scanner",
                base_url()
            )
        }));
    }
    if !cfg!(target_os = "macos") {
        return Err("starting the scanner is wired for launchd (macOS) only".into());
    }
    let root = picker_root().ok_or_else(|| {
        format!(
            "no Picker checkout found (looked for Picker/pre_surge_scanner under {}). \
             Set picker_root, or start it with: python csv_web_server.py",
            crate::config::dev_roots::dev_roots()
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let script = root.join("csv_web_server.py");
    if !script.is_file() {
        return Err(format!(
            "Picker checkout at {} has no csv_web_server.py — nothing to start",
            root.display()
        ));
    }
    let python = ensure_python_with_flask(&root).await?;

    let home = dirs::home_dir().ok_or("no home directory")?;
    let agents = home.join("Library/LaunchAgents");
    tokio::fs::create_dir_all(&agents)
        .await
        .map_err(|e| e.to_string())?;
    let plist_dst = agents.join("com.picker.stockscanner.plist");
    let plist_src = root.join("com.picker.stockscanner.plist");
    // The checkout has never shipped a plist (observed 2026-08-22: Start
    // scanner 502'd in 3ms with "no launchd job"). Write one that points at
    // the Flask entrypoint so launchd can Keep the process across daemon
    // restarts — the original reason this is not a child of permagentd.
    if plist_src.is_file() {
        tokio::fs::copy(&plist_src, &plist_dst)
            .await
            .map_err(|e| format!("could not install the launch agent: {e}"))?;
    } else {
        let logs = root.join("logs");
        tokio::fs::create_dir_all(&logs)
            .await
            .map_err(|e| e.to_string())?;
        let body = launch_plist_body(&python, &script, &root, &logs);
        tokio::fs::write(&plist_dst, body)
            .await
            .map_err(|e| format!("could not write the launch agent: {e}"))?;
    }

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

    // Bind is not instant. Wait long enough that a successful start is
    // visible on the next poll, and a failed start returns the real reason.
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(400)).await;
        if status().await.reachable {
            return Ok(format!(
                "scanner is up at {} (launchd {})",
                base_url(),
                plist_dst.display()
            ));
        }
    }
    Ok(format!(
        "asked launchd to start the scanner ({}). It takes a few seconds to bind {}. \
         If it stays down, Flask may have failed — see {}/logs/launchd.err.log",
        plist_dst.display(),
        base_url(),
        root.display()
    ))
}

fn launch_plist_body(
    python: &std::path::Path,
    script: &std::path::Path,
    workdir: &std::path::Path,
    logs: &std::path::Path,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.picker.stockscanner</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>{}</string>
  </array>
  <key>WorkingDirectory</key>
  <string>{}</string>
  <key>RunAtLoad</key>
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

/// Packages the scanner process actually imports. The checkout's
/// `requirements.txt` also lists `sqlite3` (stdlib) and `ta-lib` (needs a
/// C library) — those are not needed to bind the Flask server.
const PICKER_RUNTIME_PACKAGES: &[&str] = &[
    "flask",
    "jinja2",
    "loguru",
    "pandas",
    "numpy",
    "yfinance",
    "requests",
    "beautifulsoup4",
    "lxml",
    "pydantic",
    "python-dotenv",
];

async fn ensure_python_with_flask(root: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if let Some(py) = python_with_flask(root).await {
        return Ok(py);
    }
    let venv = root.join(".venv");
    crate::python_runtime::ensure_venv(&venv).await?;
    crate::python_runtime::pip_install(&venv, PICKER_RUNTIME_PACKAGES).await?;
    python_with_flask(root).await.ok_or_else(|| {
        format!(
            "installed Picker deps into {} but Flask still does not import",
            venv.display()
        )
    })
}

async fn python_with_flask(root: &std::path::Path) -> Option<std::path::PathBuf> {
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
        if !py.is_file() {
            continue;
        }
        if python_imports_flask(&py).await {
            return Some(py);
        }
    }
    None
}

async fn which_python() -> Result<std::path::PathBuf, String> {
    let out = run("which", &["python3"]).await?;
    let p = std::path::PathBuf::from(out);
    if p.is_file() {
        Ok(p)
    } else {
        Err("python3 not on PATH".into())
    }
}

async fn python_imports_flask(python: &std::path::Path) -> bool {
    tokio::process::Command::new(python)
        .args(["-c", "import flask"])
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

    #[test]
    fn universe_parses_mixed_separators_and_dedupes() {
        let got = parse_universe("aapl, SHOP.TO\nbrk.b; $msft  aapl");
        assert_eq!(got, vec!["AAPL", "SHOP.TO", "BRK.B", "MSFT"]);
    }

    #[test]
    fn universe_drops_non_tickers() {
        assert!(parse_universe("!!! 123").is_empty());
        assert_eq!(parse_universe("OK"), vec!["OK"]);
        assert!(parse_universe("123").is_empty());
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

    #[tokio::test]
    async fn a_malformed_close_is_rejected_by_field_not_by_the_network() {
        let err = close_trade("42", "soon", 10.0, None)
            .await
            .expect_err("bad exit_date");
        assert!(err.contains("exit_date"), "got {err}");
        let err = close_trade("42", "2026-08-21", 0.0, None)
            .await
            .expect_err("bad exit_price");
        assert!(err.contains("exit_price"), "got {err}");
        let err = close_trade("  ", "2026-08-21", 10.0, None)
            .await
            .expect_err("empty id");
        assert!(err.contains("id"), "got {err}");
    }

    #[tokio::test]
    async fn a_malformed_update_is_rejected_by_field_not_by_the_network() {
        let err = update_trade("  ", &valid_trade())
            .await
            .expect_err("empty id");
        assert!(err.contains("id"), "got {err}");
        let err = update_trade(
            "42",
            &TradeEntry {
                ticker: "  ".into(),
                ..valid_trade()
            },
        )
        .await
        .expect_err("empty ticker");
        assert!(err.contains("ticker"), "got {err}");
    }

    #[tokio::test]
    async fn delete_trade_refuses_an_empty_id() {
        let err = delete_trade("").await.expect_err("empty id");
        assert!(err.contains("id"), "got {err}");
    }

    #[test]
    fn llama_server_health_is_not_the_scanner() {
        assert!(!looks_like_picker(&serde_json::json!({"status": "ok"})));
        assert!(looks_like_picker(
            &serde_json::json!({"scan_in_progress": false, "last_scan": ""})
        ));
        assert!(looks_like_picker(&serde_json::json!({"status": "idle"})));
    }

    #[test]
    fn generated_plist_points_at_the_flask_entrypoint() {
        let body = launch_plist_body(
            std::path::Path::new("/opt/homebrew/bin/python3"),
            std::path::Path::new("/tmp/Picker/pre_surge_scanner/csv_web_server.py"),
            std::path::Path::new("/tmp/Picker/pre_surge_scanner"),
            std::path::Path::new("/tmp/Picker/pre_surge_scanner/logs"),
        );
        assert!(body.contains("com.picker.stockscanner"));
        assert!(body.contains("csv_web_server.py"));
        assert!(body.contains("/opt/homebrew/bin/python3"));
        assert!(body.contains("WorkingDirectory"));
    }

    #[test]
    fn the_default_base_is_loopback() {
        // The scanner has no auth and can modify trades; it must not be
        // addressed off-machine by default.
        assert!(DEFAULT_BASE.starts_with("http://127.0.0.1"));
    }

    #[test]
    fn picks_from_body_reads_live_results_shape() {
        let v = serde_json::json!({
            "results": [
                {"ticker": "ENB", "rsi": "52.1", "total_score": 18, "rank": 1}
            ]
        });
        let picks = picks_from_body(&v);
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0]["ticker"], "ENB");
    }

    #[test]
    fn picks_from_body_reads_legacy_top_picks_shape() {
        let v = serde_json::json!({"picks": [{"ticker": "AAPL"}]});
        assert_eq!(picks_from_body(&v).len(), 1);
        let bare = serde_json::json!([{"ticker": "MSFT"}]);
        assert_eq!(picks_from_body(&bare).len(), 1);
    }

    #[test]
    fn parse_money_strips_currency_and_commas() {
        assert_eq!(parse_money("$1,240.00"), Some(1240.0));
        assert_eq!(parse_money("N/A"), None);
        assert_eq!(parse_money("52.1%"), Some(52.1));
    }

    #[test]
    fn parse_trade_row_reads_picker_shape() {
        let v = serde_json::json!({
            "id": 7,
            "ticker": "enb",
            "company_name": "Enbridge",
            "entry_date": "2026-01-15",
            "entry_price": 52.1,
            "shares": 100
        });
        let t = parse_trade_row(&v).unwrap();
        assert_eq!(t.ticker, "ENB");
        assert_eq!(t.id, "7");
        assert!(t.exit_date.is_none());
    }
}
