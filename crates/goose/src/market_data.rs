//! Market data — live quotes with no setup, plus optional fundamentals.
//!
//! The Financier's research half has to work for a user who has no trading
//! stack of their own, so quotes reach Yahoo Finance's public endpoints
//! directly: no API key, no account, no local service. Fundamentals use the
//! authenticated financialdatasets.ai API only when its optional key exists.
//!
//! ## What that costs, stated plainly
//!
//! The Yahoo quote endpoints are **not a supported API**. They are the ones Yahoo's own
//! web client calls; they are rate-limited, occasionally require a crumb/cookie
//! handshake, and have changed shape before without notice. So:
//!
//!   * every field is optional and read defensively — a response that changes
//!     shape yields a thinner answer, never a panic and never a wrong number;
//!   * a failure is reported as a failure. A quote the agent could not fetch
//!     must never be answered from the model's memory, which is months stale
//!     and confidently wrong;
//!   * nothing here is cached across calls. A price is only a price at the
//!     moment it was read, and its timestamp travels with it.
//!
//! The three bullets bind the fundamentals path too, and it carries one more
//! obligation: an absent key and a rejected key are separate states, reported
//! separately. "You have not set one" and "the one you set was refused" have
//! different fixes, and the key's VALUE never appears in a message, a log, or
//! a URL — only the name of the key does.
//!
//! This is deliberately a *research* source, not an execution source. Nothing
//! in this module can place an order.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::config::Config;

const QUOTE_BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const TIMEOUT: Duration = Duration::from_secs(12);

/// financialdatasets.ai key. Same name their own MCP server uses, so a
/// user who already has one does not have to learn a second name.
pub const FUNDAMENTALS_KEY: &str = "FINANCIAL_DATASETS_API_KEY";
const FUNDAMENTALS_URL: &str = "https://api.financialdatasets.ai/financials";

/// "No key configured" and "key configured, call failed" are different
/// states and the user must be told which one they are in. Conflating them
/// is the exact class this codebase was audited for.
#[derive(Debug)]
pub enum FundamentalsError {
    NotConfigured,
    Failed(String),
}

/// One statement kind: where it lives in the response, what it is called to the
/// user, and the line items read out of it as (source field, human label). The
/// source may rename or drop any of them, so each is read defensively and a
/// field that is absent is simply not read — never rendered as zero.
struct StatementSpec {
    source: &'static str,
    label: &'static str,
    line_items: &'static [(&'static str, &'static str)],
}

const STATEMENTS: &[StatementSpec] = &[
    StatementSpec {
        source: "income_statements",
        label: "Income",
        line_items: &[
            ("revenue", "revenue"),
            ("operating_income", "operating income"),
            ("net_income", "net income"),
            ("earnings_per_share", "earnings per share"),
        ],
    },
    StatementSpec {
        source: "balance_sheets",
        label: "Balance sheet",
        line_items: &[
            ("total_assets", "assets"),
            ("total_liabilities", "liabilities"),
            ("shareholders_equity", "shareholders' equity"),
        ],
    },
    StatementSpec {
        source: "cash_flow_statements",
        label: "Cash flow",
        line_items: &[
            ("net_cash_flow_from_operations", "operating cash flow"),
            ("free_cash_flow", "free cash flow"),
        ],
    },
];

#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub label: &'static str,
    pub report_period: Option<String>,
    /// Only the line items that were actually readable.
    pub line_items: Vec<(&'static str, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fundamentals {
    pub symbol: String,
    pub statements: Vec<Statement>,
}

/// The env var wins over the stored secret, and a blank value counts as
/// absent — a key set to the empty string is not a key, and reporting it as
/// one would send the user looking for a configuration problem they do not
/// have. Pure, and the stored secret is read lazily, so precedence and the
/// blank rule are testable without touching the process environment or the
/// keychain.
fn resolve_fundamentals_key(
    env_value: Option<String>,
    config_value: impl FnOnce() -> Option<String>,
) -> Option<String> {
    env_value
        .and_then(non_empty)
        .or_else(|| config_value().and_then(non_empty))
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn fundamentals_key() -> Option<String> {
    resolve_fundamentals_key(std::env::var(FUNDAMENTALS_KEY).ok(), || {
        Config::global().get_secret::<String>(FUNDAMENTALS_KEY).ok()
    })
}

/// Presence only — the value never leaves this module.
pub fn fundamentals_configured() -> bool {
    fundamentals_key().is_some()
}

/// Right-align per-lot close series and sum `(close − entry) × shares`.
/// Empty or a single point is not a trend; callers omit the chart.
pub fn net_unrealized_trend(lots: &[(Vec<f64>, f64, f64)]) -> Vec<f64> {
    let n = lots.iter().map(|(c, _, _)| c.len()).min().unwrap_or(0);
    if n < 2 {
        return Vec::new();
    }
    let mut out = vec![0.0; n];
    for (closes, entry, shares) in lots {
        let skip = closes.len() - n;
        for (i, px) in closes.iter().skip(skip).enumerate() {
            out[i] += (px - entry) * shares;
        }
    }
    const MAX_POINTS: usize = 60;
    if out.len() <= MAX_POINTS {
        return out;
    }
    let step = out.len() as f64 / MAX_POINTS as f64;
    (0..MAX_POINTS)
        .map(|i| out[(i as f64 * step).floor() as usize])
        .collect()
}

pub async fn fundamentals(
    symbol: &str,
    period: &str,
    limit: u8,
) -> Result<Fundamentals, FundamentalsError> {
    // Resolved first, and the only source of the auth header: no key means the
    // function returns before a client or a URL exists, so there is no path on
    // which an unauthenticated request is attempted.
    let key = fundamentals_key().ok_or(FundamentalsError::NotConfigured)?;
    let symbol = normalize_symbol(symbol).map_err(FundamentalsError::Failed)?;
    // `^` is legal in a ticker (`^GSPC`) and must be percent-encoded, so the
    // query is built by the URL parser rather than string-formatted.
    let limit = limit.to_string();
    let url = reqwest::Url::parse_with_params(
        FUNDAMENTALS_URL,
        &[
            ("ticker", symbol.as_str()),
            ("period", period),
            ("limit", limit.as_str()),
        ],
    )
    .map_err(|e| FundamentalsError::Failed(format!("could not build the request URL: {e}")))?;
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| FundamentalsError::Failed(e.to_string()))?;
    let response = client
        .get(url)
        .header("X-API-KEY", &key)
        .send()
        .await
        .map_err(|e| {
            FundamentalsError::Failed(format!("could not reach financialdatasets.ai: {e}"))
        })?;
    let status = response.status();
    let text = response.text().await.map_err(|e| {
        FundamentalsError::Failed(format!(
            "financialdatasets.ai returned an unreadable response: {e}"
        ))
    })?;
    if !status.is_success() {
        return Err(FundamentalsError::Failed(fundamentals_failure_message(
            status, &text, &key,
        )));
    }
    let body: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        FundamentalsError::Failed(format!(
            "financialdatasets.ai returned unreadable JSON: {e}"
        ))
    })?;
    parse_fundamentals(&body, &symbol).map_err(FundamentalsError::Failed)
}

/// A non-2xx from the fundamentals source, put into words. A rejected
/// credential is called out as such: "the key you set was refused" and "you
/// set no key" are different problems with different fixes, and this codebase
/// was audited for conflating exactly that pair. The key is stripped from the
/// echoed body — some APIs quote the offending credential back at you, and
/// this string reaches the model and the transcript.
fn fundamentals_failure_message(status: reqwest::StatusCode, body: &str, key: &str) -> String {
    let redacted = body.replace(key, "[redacted]");
    let detail = redacted.trim();
    let detail = if detail.is_empty() {
        "no detail given".to_string()
    } else {
        detail.chars().take(500).collect()
    };
    if matches!(status.as_u16(), 401 | 403) {
        format!("financialdatasets.ai REJECTED the configured key ({status}): {detail}")
    } else {
        format!("financialdatasets.ai answered {status}: {detail}")
    }
}

pub fn parse_fundamentals(body: &serde_json::Value, symbol: &str) -> Result<Fundamentals, String> {
    let financials = body
        .get("financials")
        .ok_or("financialdatasets.ai returned no financials object")?;
    let statements = STATEMENTS
        .iter()
        .flat_map(|spec| {
            financials
                .get(spec.source)
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .map(move |row| Statement {
                    label: spec.label,
                    report_period: row
                        .get("report_period")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                    line_items: spec
                        .line_items
                        .iter()
                        .filter_map(|(field, label)| {
                            row.get(field)
                                .and_then(|value| value.as_f64())
                                .map(|value| (*label, value))
                        })
                        .collect(),
                })
        })
        .collect();
    Ok(Fundamentals {
        symbol: symbol.to_string(),
        statements,
    })
}

/// A human-readable rendering, for the agent to narrate. Absent line items are
/// omitted rather than rendered as zero, and a statement whose every line item
/// was unreadable says so — an empty row would read as "the company reported
/// nothing", which is a different and false claim.
pub fn describe_fundamentals(f: &Fundamentals) -> String {
    fn section(lines: &mut Vec<String>, label: &str, period: Option<&str>, values: Vec<String>) {
        let period = period.unwrap_or("period unavailable");
        if values.is_empty() {
            lines.push(format!(
                "{label} {period}: no line items were readable in the response"
            ));
        } else {
            lines.push(format!("{label} {period}: {}", values.join(", ")));
        }
    }

    let mut lines = vec![format!(
        "Fundamentals for {}, as reported, from financialdatasets.ai",
        f.symbol
    )];
    for statement in &f.statements {
        section(
            &mut lines,
            statement.label,
            statement.report_period.as_deref(),
            statement
                .line_items
                .iter()
                .map(|(label, value)| format!("{label} {value}"))
                .collect(),
        );
    }
    if lines.len() == 1 {
        lines.push(
            "No statements came back in a readable shape. That may mean the source has \
             nothing for this ticker, or that its response changed shape — from here the \
             two are indistinguishable, so say so. It is NOT evidence the company reported \
             nothing, and must not be filled in from memory."
                .into(),
        );
    }
    lines.join("\n")
}

/// Yahoo rejects requests without a browser-ish agent.
const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/120.0 Safari/537.36";

/// A quote as read at a moment in time. Every field optional: the shape is not
/// contractual, and a missing field is honest where a zero would be a lie.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Quote {
    pub symbol: String,
    pub name: Option<String>,
    pub currency: Option<String>,
    pub exchange: Option<String>,
    pub price: Option<f64>,
    pub previous_close: Option<f64>,
    /// Change from the previous close, absolute and percent. Derived here so
    /// the two can never disagree with `price`.
    pub change: Option<f64>,
    pub change_percent: Option<f64>,
    pub day_high: Option<f64>,
    pub day_low: Option<f64>,
    pub fifty_two_week_high: Option<f64>,
    pub fifty_two_week_low: Option<f64>,
    pub volume: Option<u64>,
    /// When the quote was stamped by the exchange, ISO-8601 UTC.
    pub quoted_at: Option<String>,
    /// True when the market was closed at the time of reading.
    pub market_closed: bool,
}

async fn chart(symbol: &str, range: &str) -> Result<serde_json::Value, String> {
    let symbol = normalize_symbol(symbol)?;
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{QUOTE_BASE}/{symbol}?interval=1d&range={range}");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("could not reach the market data source: {e}"))?;
    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("market data source answered something unreadable: {e}"))?;
    if !status.is_success() {
        // Yahoo puts a usable reason in the envelope on 404 (unknown symbol).
        let detail = body
            .pointer("/chart/error/description")
            .and_then(|d| d.as_str())
            .unwrap_or("no detail given");
        return Err(format!("market data source answered {status}: {detail}"));
    }
    Ok(body)
}

/// Fetch one symbol. `Err` means we could not get an answer — the caller must
/// say so rather than substituting anything it remembers.
pub async fn quote(symbol: &str) -> Result<Quote, String> {
    let symbol = normalize_symbol(symbol)?;
    let body = chart(&symbol, "5d").await?;
    parse_quote(&body, &symbol)
}

/// Daily closes, oldest → newest, for the loop-engineering gate and RSI.
/// `range` is a Yahoo chart range (`1y`, `6mo`). Missing bars are dropped,
/// never filled with zero.
pub async fn daily_closes(symbol: &str, range: &str) -> Result<Vec<f64>, String> {
    let symbol = normalize_symbol(symbol)?;
    let body = chart(&symbol, range).await?;
    parse_closes(&body)
}

/// Pull a close series out of a chart response. Testable without the network.
pub fn parse_closes(body: &serde_json::Value) -> Result<Vec<f64>, String> {
    let closes = body
        .pointer("/chart/result/0/indicators/quote/0/close")
        .and_then(|v| v.as_array())
        .ok_or("the market data source returned no daily closes")?;
    let out: Vec<f64> = closes.iter().filter_map(|v| v.as_f64()).collect();
    if out.len() < 16 {
        return Err("not enough daily closes came back to compute a series".into());
    }
    Ok(out)
}

/// One split- and dividend-adjusted daily bar, oldest → newest in a series.
///
/// `close` is Yahoo's `adjclose`; `open`/`high`/`low` are the raw quote values
/// scaled by the same `adjclose/close` factor, so the four prices sit on ONE
/// consistent basis. Mixing bases turns a 2-for-1 split into a spurious
/// Donchian breakdown and a spurious death cross on the same day.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DailyBar {
    /// Exchange timestamp for the bar, epoch seconds.
    pub epoch_seconds: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    /// Split-adjusted share volume. Zero is a real reading (a halted or
    /// untraded day) and is preserved here; volume *statistics* skip it
    /// rather than averaging a zero in.
    pub volume: u64,
}

/// Why a bar series could not be built. Typed, because the callers must be
/// able to tell "this symbol has no high/low" from "this symbol is too young"
/// from "the network was down" — every one of those has a different fix, and
/// collapsing them into a string invites a silent close-only substitution.
#[derive(Debug, Clone, PartialEq)]
pub enum BarsError {
    /// Could not reach or read the source.
    Fetch(String),
    /// The envelope had no result for the symbol.
    NoResult,
    /// A required series was absent from the payload — `"high"`, `"volume"`, …
    MissingSeries(&'static str),
    /// A series came back a different length from the timestamp axis, so the
    /// bars cannot be aligned. Guessing the alignment would silently shift
    /// every window by an unknown offset.
    LengthMismatch {
        field: &'static str,
        got: usize,
        want: usize,
    },
    /// No `adjclose` series. Raw prices are not usable for these indicators.
    MissingAdjustedClose,
    /// A bar claimed a high below its own low: the payload is corrupt, not thin.
    InconsistentBar { index: usize },
    /// Every bar was unreadable (all-null arrays, e.g. a close-only feed).
    NoUsableBars,
}

impl std::fmt::Display for BarsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fetch(e) => write!(f, "could not fetch daily bars: {e}"),
            Self::NoResult => write!(
                f,
                "the market data source returned no result for that symbol"
            ),
            Self::MissingSeries(field) => write!(
                f,
                "the market data source returned no daily {field} series — true high/low are \
                 required and must not be substituted with closes"
            ),
            Self::LengthMismatch { field, got, want } => write!(
                f,
                "the daily {field} series had {got} points against {want} timestamps, so the \
                 bars cannot be aligned"
            ),
            Self::MissingAdjustedClose => write!(
                f,
                "the market data source returned no adjusted closes — raw prices would turn a \
                 split into a false breakdown"
            ),
            Self::InconsistentBar { index } => {
                write!(f, "daily bar {index} has a high below its own low")
            }
            Self::NoUsableBars => write!(f, "no readable daily bars came back"),
        }
    }
}

impl std::error::Error for BarsError {}

/// The default history window for indicator work.
///
/// `chan_pos_252` needs 252 bars *before* the current one, i.e. 253; `"1y"`
/// returns about 252 total and would fail on the first holiday. Yahoo accepts
/// an explicit day count, so ask for 300 — F0 §6's "fetch 300 to survive
/// holidays, halts and gaps".
pub const DEFAULT_BARS_RANGE: &str = "300d";

/// Adjusted daily OHLCV, oldest → newest.
///
/// This is the input the indicator engine requires; [`daily_closes`] remains
/// the close-only path for the loop gate and RSI, unchanged.
pub async fn daily_bars(symbol: &str, range: &str) -> Result<Vec<DailyBar>, BarsError> {
    let symbol = normalize_symbol(symbol).map_err(BarsError::Fetch)?;
    let body = chart(&symbol, range).await.map_err(BarsError::Fetch)?;
    parse_bars(&body)
}

/// Pull an adjusted OHLCV series out of a chart response. Testable without
/// the network.
///
/// Bars with any null price, a null volume, or a zero/absent close (which
/// would make the adjustment factor undefined) are dropped — Yahoo emits
/// those for days the symbol did not trade. A bar is never *repaired*: no
/// forward fill, no zero fill, no close-for-high.
pub fn parse_bars(body: &serde_json::Value) -> Result<Vec<DailyBar>, BarsError> {
    let result = body.pointer("/chart/result/0").ok_or(BarsError::NoResult)?;
    let stamps = result
        .get("timestamp")
        .and_then(|v| v.as_array())
        .ok_or(BarsError::MissingSeries("timestamp"))?;
    let quote = result
        .pointer("/indicators/quote/0")
        .ok_or(BarsError::NoResult)?;

    let series = |field: &'static str| -> Result<&Vec<serde_json::Value>, BarsError> {
        let arr = quote
            .get(field)
            .and_then(|v| v.as_array())
            .ok_or(BarsError::MissingSeries(field))?;
        if arr.len() != stamps.len() {
            return Err(BarsError::LengthMismatch {
                field,
                got: arr.len(),
                want: stamps.len(),
            });
        }
        Ok(arr)
    };

    let opens = series("open")?;
    let highs = series("high")?;
    let lows = series("low")?;
    let closes = series("close")?;
    let volumes = series("volume")?;

    let adj = result
        .pointer("/indicators/adjclose/0/adjclose")
        .and_then(|v| v.as_array())
        .ok_or(BarsError::MissingAdjustedClose)?;
    if adj.len() != stamps.len() {
        return Err(BarsError::LengthMismatch {
            field: "adjclose",
            got: adj.len(),
            want: stamps.len(),
        });
    }

    let mut out = Vec::with_capacity(stamps.len());
    for i in 0..stamps.len() {
        let (Some(t), Some(o), Some(h), Some(l), Some(c), Some(a), Some(v)) = (
            stamps[i].as_i64(),
            opens[i].as_f64(),
            highs[i].as_f64(),
            lows[i].as_f64(),
            closes[i].as_f64(),
            adj[i].as_f64(),
            volumes[i].as_u64(),
        ) else {
            continue;
        };
        if c == 0.0 || !c.is_finite() || !a.is_finite() {
            continue;
        }
        // One factor for the whole bar: adjusted close over raw close.
        let factor = a / c;
        let bar = DailyBar {
            epoch_seconds: t,
            open: o * factor,
            high: h * factor,
            low: l * factor,
            close: a,
            volume: v,
        };
        if bar.high < bar.low {
            return Err(BarsError::InconsistentBar { index: i });
        }
        out.push(bar);
    }
    if out.is_empty() {
        return Err(BarsError::NoUsableBars);
    }
    Ok(out)
}

/// Pull a [`Quote`] out of a chart response. Separated from the request so the
/// shape handling is testable against captured payloads.
pub fn parse_quote(body: &serde_json::Value, symbol: &str) -> Result<Quote, String> {
    let result = body
        .pointer("/chart/result/0")
        .ok_or("the market data source returned no result for that symbol")?;
    let meta = result.get("meta").unwrap_or(&serde_json::Value::Null);

    let f = |key: &str| meta.get(key).and_then(|v| v.as_f64());
    let price = f("regularMarketPrice");
    let previous_close = f("chartPreviousClose").or_else(|| f("previousClose"));

    let (change, change_percent) = match (price, previous_close) {
        (Some(p), Some(pc)) if pc != 0.0 => (Some(p - pc), Some((p - pc) / pc * 100.0)),
        _ => (None, None),
    };

    Ok(Quote {
        symbol: meta
            .get("symbol")
            .and_then(|v| v.as_str())
            .unwrap_or(symbol)
            .to_string(),
        name: meta
            .get("longName")
            .or_else(|| meta.get("shortName"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        currency: meta
            .get("currency")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        exchange: meta
            .get("fullExchangeName")
            .or_else(|| meta.get("exchangeName"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        price,
        previous_close,
        change,
        change_percent,
        day_high: f("regularMarketDayHigh"),
        day_low: f("regularMarketDayLow"),
        fifty_two_week_high: f("fiftyTwoWeekHigh"),
        fifty_two_week_low: f("fiftyTwoWeekLow"),
        volume: meta.get("regularMarketVolume").and_then(|v| v.as_u64()),
        quoted_at: meta
            .get("regularMarketTime")
            .and_then(|v| v.as_i64())
            .and_then(epoch_to_iso),
        market_closed: meta
            .get("marketState")
            .and_then(|v| v.as_str())
            .map(|s| s != "REGULAR")
            .unwrap_or(false),
    })
}

fn epoch_to_iso(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0).map(|dt| dt.to_rfc3339())
}

/// Reject anything that is not plausibly a ticker before it becomes a URL
/// path segment. Tickers are letters, digits, `.`, `-` and `^` (indices);
/// nothing here needs escaping once that holds.
fn normalize_symbol(raw: &str) -> Result<String, String> {
    let s = raw.trim().to_uppercase();
    if s.is_empty() {
        return Err("a ticker symbol is required".into());
    }
    if s.len() > 20 {
        return Err("that does not look like a ticker symbol".into());
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '^' | '='))
    {
        return Err(format!(
            "'{raw}' is not a ticker symbol — letters, digits, '.', '-', '^' and '=' only"
        ));
    }
    Ok(s)
}

/// A human-readable rendering, for the agent to narrate. Absent fields are
/// omitted rather than rendered as zero.
pub fn describe(q: &Quote) -> String {
    let mut lines = vec![format!(
        "{}{}",
        q.symbol,
        q.name
            .as_deref()
            .map(|n| format!(" — {n}"))
            .unwrap_or_default()
    )];
    if let Some(p) = q.price {
        let cur = q.currency.as_deref().unwrap_or("");
        let mv = match (q.change, q.change_percent) {
            (Some(c), Some(pc)) => format!(" ({}{:.2}, {}{:.2}%)", sign(c), c, sign(pc), pc),
            _ => String::new(),
        };
        lines.push(format!("Price: {p:.2} {cur}{mv}"));
    }
    if let (Some(lo), Some(hi)) = (q.day_low, q.day_high) {
        lines.push(format!("Day range: {lo:.2} – {hi:.2}"));
    }
    if let (Some(lo), Some(hi)) = (q.fifty_two_week_low, q.fifty_two_week_high) {
        lines.push(format!("52-week range: {lo:.2} – {hi:.2}"));
    }
    if let Some(v) = q.volume {
        lines.push(format!("Volume: {v}"));
    }
    if let Some(t) = q.quoted_at.as_deref() {
        lines.push(format!(
            "As of: {t}{}",
            if q.market_closed {
                " (market closed)"
            } else {
                ""
            }
        ));
    }
    lines.join("\n")
}

fn sign(v: f64) -> &'static str {
    if v >= 0.0 {
        "+"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart(meta: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "chart": { "result": [ { "meta": meta } ], "error": null } })
    }

    #[test]
    fn symbols_are_validated_before_they_become_a_url() {
        assert_eq!(normalize_symbol(" aapl ").unwrap(), "AAPL");
        assert_eq!(normalize_symbol("shop.to").unwrap(), "SHOP.TO");
        assert_eq!(normalize_symbol("^GSPC").unwrap(), "^GSPC");
        assert_eq!(normalize_symbol("BRK-B").unwrap(), "BRK-B");
        assert_eq!(normalize_symbol("CAD=X").unwrap(), "CAD=X");
        for bad in [
            "",
            "   ",
            "../../etc/passwd",
            "AAPL?x=1",
            "a b",
            "AAPL/../X",
        ] {
            assert!(normalize_symbol(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn a_quote_is_read_out_of_the_chart_envelope() {
        let v = chart(serde_json::json!({
            "symbol": "AAPL", "longName": "Apple Inc.", "currency": "USD",
            "fullExchangeName": "NasdaqGS", "regularMarketPrice": 210.0,
            "chartPreviousClose": 200.0, "regularMarketDayHigh": 212.0,
            "regularMarketDayLow": 205.0, "fiftyTwoWeekHigh": 260.0,
            "fiftyTwoWeekLow": 164.0, "regularMarketVolume": 51_000_000,
            "regularMarketTime": 1_754_000_000i64, "marketState": "REGULAR"
        }));
        let q = parse_quote(&v, "AAPL").unwrap();
        assert_eq!(q.name.as_deref(), Some("Apple Inc."));
        assert_eq!(q.price, Some(210.0));
        // Derived here so they can never disagree with the price.
        assert_eq!(q.change, Some(10.0));
        assert_eq!(q.change_percent, Some(5.0));
        assert!(!q.market_closed);
        assert!(q.quoted_at.is_some());
    }

    #[test]
    fn a_thinner_payload_yields_a_thinner_answer_not_a_wrong_one() {
        // These endpoints are not a contract. A response that loses fields must
        // lose the corresponding claims, never substitute zero.
        let q = parse_quote(&chart(serde_json::json!({ "symbol": "XYZ" })), "XYZ").unwrap();
        assert_eq!(q.symbol, "XYZ");
        assert_eq!(q.price, None);
        assert_eq!(q.change, None);
        assert_eq!(q.change_percent, None);
        assert_eq!(q.volume, None);
        assert!(describe(&q).contains("XYZ"), "still names the symbol");
    }

    #[test]
    fn no_previous_close_means_no_change_rather_than_zero_change() {
        let q = parse_quote(
            &chart(serde_json::json!({ "symbol": "X", "regularMarketPrice": 10.0 })),
            "X",
        )
        .unwrap();
        assert_eq!(q.price, Some(10.0));
        assert_eq!(q.change, None, "unknown movement is not flat movement");
        // A zero previous close must not divide.
        let q0 = parse_quote(
            &chart(serde_json::json!({
                "symbol": "X", "regularMarketPrice": 10.0, "chartPreviousClose": 0.0
            })),
            "X",
        )
        .unwrap();
        assert_eq!(q0.change_percent, None);
    }

    #[test]
    fn an_empty_result_is_an_error_not_an_empty_quote() {
        let v = serde_json::json!({ "chart": { "result": [], "error": null } });
        assert!(parse_quote(&v, "NOPE").is_err());
    }

    #[test]
    fn a_closed_market_says_so() {
        let q = parse_quote(
            &chart(serde_json::json!({ "symbol": "X", "marketState": "CLOSED" })),
            "X",
        )
        .unwrap();
        assert!(q.market_closed);
    }

    #[test]
    fn describe_omits_what_it_does_not_know() {
        let q = Quote {
            symbol: "X".into(),
            ..Default::default()
        };
        let text = describe(&q);
        assert!(
            !text.contains("0.00"),
            "absent values must not render as zero"
        );
        assert!(!text.contains("Volume"));
    }

    #[test]
    fn missing_fundamentals_key_is_resolved_before_any_request() {
        // The pure seam avoids mutating the process environment, which is
        // global to the whole test binary. `fundamentals` resolves through it
        // before a client or a URL exists, so `None` here is the no-request path.
        assert_eq!(resolve_fundamentals_key(None, || None), None);
        assert_eq!(
            resolve_fundamentals_key(Some("   ".into()), || Some("".into())),
            None
        );
        // A blank env var must not shadow a real stored secret.
        assert_eq!(
            resolve_fundamentals_key(Some("".into()), || Some("stored".into())),
            Some("stored".into())
        );
        // Env wins, and the stored secret is not even read.
        assert_eq!(
            resolve_fundamentals_key(Some(" env ".into()), || {
                panic!("the stored secret must not be read when the env var is set")
            }),
            Some("env".into())
        );
    }

    /// A key that was configured and refused is a different report from no key
    /// at all: the first is a credential the user must fix, the second is a
    /// feature they never turned on.
    #[test]
    fn rejected_key_reads_differently_from_no_key() {
        let rejected = fundamentals_failure_message(
            reqwest::StatusCode::UNAUTHORIZED,
            "{\"detail\":\"bad key\"}",
            "s3cret",
        );
        assert!(rejected.contains("REJECTED"), "{rejected}");
        let other = fundamentals_failure_message(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "boom",
            "s3cret",
        );
        assert!(!other.contains("REJECTED"), "{other}");
        // The key never travels in an error message, however the source echoes it.
        let echoed = fundamentals_failure_message(
            reqwest::StatusCode::BAD_REQUEST,
            "unknown key s3cret supplied",
            "s3cret",
        );
        assert!(!echoed.contains("s3cret"), "{echoed}");
    }

    #[test]
    fn captured_fundamentals_shape_parses_defensively() {
        let full = serde_json::json!({ "financials": {
            "income_statements": [{
                "ticker": "ACME", "report_period": "2025-12-31", "fiscal_period": "FY",
                "period": "annual", "revenue": 100.0, "operating_income": 20.0,
                "net_income": 12.0, "earnings_per_share": 1.5
            }],
            "balance_sheets": [{
                "ticker": "ACME", "report_period": "2025-12-31", "fiscal_period": "FY",
                "period": "annual", "total_assets": 300.0, "total_liabilities": 120.0,
                "shareholders_equity": 180.0
            }],
            "cash_flow_statements": [{
                "ticker": "ACME", "report_period": "2025-12-31", "fiscal_period": "FY",
                "period": "annual", "net_cash_flow_from_operations": 30.0,
                "free_cash_flow": 18.0
            }]
        }});
        let parsed = parse_fundamentals(&full, "ACME").unwrap();
        assert_eq!(
            parsed.statements[0].line_items,
            vec![
                ("revenue", 100.0),
                ("operating income", 20.0),
                ("net income", 12.0),
                ("earnings per share", 1.5),
            ]
        );
        assert_eq!(
            parsed.statements[1].line_items,
            vec![
                ("assets", 300.0),
                ("liabilities", 120.0),
                ("shareholders' equity", 180.0),
            ]
        );
        assert_eq!(
            parsed.statements[2].line_items,
            vec![("operating cash flow", 30.0), ("free cash flow", 18.0)]
        );

        let thin = serde_json::json!({ "financials": {
            "income_statements": [{ "sales_renamed": 100.0 }]
        }});
        let parsed = parse_fundamentals(&thin, "ACME").unwrap();
        assert!(parsed.statements[0].line_items.is_empty());
        assert!(!parsed
            .statements
            .iter()
            .any(|statement| statement.label == "Balance sheet"));
    }

    #[test]
    fn fundamentals_description_does_not_fabricate_missing_values() {
        let text = describe_fundamentals(&Fundamentals {
            symbol: "ACME".into(),
            statements: vec![Statement {
                label: "Income",
                report_period: Some("2025-12-31".into()),
                line_items: Vec::new(),
            }],
        });
        assert!(
            text.contains("Fundamentals for ACME, as reported, from financialdatasets.ai"),
            "{text}"
        );
        assert!(!text.contains("revenue"), "{text}");
        assert!(!text.contains("income 0"), "{text}");
        // A row whose every line item was unreadable says so. A bare
        // "Income 2025-12-31:" would read as a company that reported nothing.
        assert!(text.contains("no line items were readable"), "{text}");

        // Nothing readable at all must not render as "the company reported nothing".
        let empty = describe_fundamentals(&Fundamentals {
            symbol: "ACME".into(),
            statements: Vec::new(),
        });
        assert!(empty.contains("NOT evidence"), "{empty}");
    }

    #[test]
    fn parse_closes_drops_null_bars_and_refuses_a_thin_series() {
        let body = serde_json::json!({
            "chart": { "result": [{ "indicators": { "quote": [{
                "close": [10.0, null, 11.0, 12.0]
            }]}}]}
        });
        assert!(parse_closes(&body).is_err(), "four readable? no — only 3");
        let closes: Vec<f64> = (0..20).map(|i| 10.0 + i as f64).collect();
        let fat = serde_json::json!({
            "chart": { "result": [{ "indicators": { "quote": [{ "close": closes }]}}]}
        });
        assert_eq!(parse_closes(&fat).unwrap().len(), 20);
    }

    /// A chart envelope carrying the full OHLCV + adjclose shape Yahoo sends.
    fn ohlcv(
        stamps: Vec<i64>,
        open: Vec<serde_json::Value>,
        high: Vec<serde_json::Value>,
        low: Vec<serde_json::Value>,
        close: Vec<serde_json::Value>,
        volume: Vec<serde_json::Value>,
        adj: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        serde_json::json!({ "chart": { "result": [{
            "timestamp": stamps,
            "indicators": {
                "quote": [{ "open": open, "high": high, "low": low,
                            "close": close, "volume": volume }],
                "adjclose": [{ "adjclose": adj }]
            }
        }], "error": null }})
    }

    #[test]
    fn bars_carry_the_high_low_and_volume_parse_closes_throws_away() {
        let body = ohlcv(
            vec![1_700_000_000, 1_700_086_400],
            vec![10.0.into(), 11.0.into()],
            vec![12.0.into(), 13.0.into()],
            vec![9.0.into(), 10.5.into()],
            vec![11.0.into(), 12.0.into()],
            vec![1_000.into(), 2_500.into()],
            vec![11.0.into(), 12.0.into()], // no adjustment: adjclose == close
        );
        let bars = parse_bars(&body).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].epoch_seconds, 1_700_000_000);
        assert_eq!(bars[0].high, 12.0);
        assert_eq!(bars[0].low, 9.0);
        assert_eq!(bars[1].volume, 2_500);
        // And the close-only path is untouched by any of this.
        assert_eq!(
            parse_closes(&ohlcv(
                (0..20).collect(),
                (0..20).map(|i| (i as f64).into()).collect(),
                (0..20).map(|i| (i as f64 + 1.0).into()).collect(),
                (0..20).map(|i| (i as f64 - 1.0).into()).collect(),
                (0..20).map(|i| (i as f64).into()).collect(),
                (0..20).map(|_| 100.into()).collect(),
                (0..20).map(|i| (i as f64).into()).collect(),
            ))
            .unwrap()
            .len(),
            20
        );
    }

    #[test]
    fn every_price_in_a_bar_sits_on_the_adjusted_basis() {
        // A 10% dividend adjustment: adjclose 90 against a raw close of 100
        // scales the whole bar by 0.9. Mixing an adjusted close with a raw
        // high would invent a range the stock never traded.
        let body = ohlcv(
            vec![1_700_000_000],
            vec![95.0.into()],
            vec![110.0.into()],
            vec![90.0.into()],
            vec![100.0.into()],
            vec![1_000.into()],
            vec![90.0.into()],
        );
        let b = parse_bars(&body).unwrap()[0];
        assert_eq!(b.close, 90.0);
        assert!((b.high - 99.0).abs() < 1e-12, "110 * 0.9 = 99");
        assert!((b.low - 81.0).abs() < 1e-12, "90 * 0.9 = 81");
        assert!((b.open - 85.5).abs() < 1e-12, "95 * 0.9 = 85.5");
        // The adjusted bar still brackets its own close.
        assert!(b.high >= b.close && b.low <= b.close);
    }

    #[test]
    fn a_close_only_payload_fails_loudly_instead_of_substituting_closes() {
        // Exactly the shape `parse_closes` is happy with: no high, no low, no
        // volume. The indicator path must refuse it rather than pour closes
        // into the high and low fields.
        let closes: Vec<f64> = (0..300).map(|i| 10.0 + i as f64).collect();
        let close_only = serde_json::json!({
            "chart": { "result": [{
                "timestamp": (0..300).collect::<Vec<i64>>(),
                "indicators": { "quote": [{ "close": closes }] }
            }]}
        });
        assert_eq!(
            parse_bars(&close_only).unwrap_err(),
            BarsError::MissingSeries("open")
        );
        assert!(
            parse_closes(&close_only).is_ok(),
            "unchanged for its callers"
        );

        // Present-but-all-null highs are the same failure wearing a hat.
        let nulled = ohlcv(
            vec![1, 2],
            vec![1.0.into(), 2.0.into()],
            vec![serde_json::Value::Null, serde_json::Value::Null],
            vec![1.0.into(), 2.0.into()],
            vec![1.0.into(), 2.0.into()],
            vec![10.into(), 10.into()],
            vec![1.0.into(), 2.0.into()],
        );
        assert_eq!(parse_bars(&nulled).unwrap_err(), BarsError::NoUsableBars);
    }

    #[test]
    fn unadjusted_prices_are_refused_rather_than_used_raw() {
        let body = serde_json::json!({
            "chart": { "result": [{
                "timestamp": [1, 2],
                "indicators": { "quote": [{
                    "open": [1.0, 2.0], "high": [2.0, 3.0], "low": [0.5, 1.5],
                    "close": [1.0, 2.0], "volume": [10, 20]
                }]}
            }]}
        });
        assert_eq!(
            parse_bars(&body).unwrap_err(),
            BarsError::MissingAdjustedClose
        );
    }

    #[test]
    fn misaligned_series_are_refused_rather_than_guessed_into_place() {
        let body = ohlcv(
            vec![1, 2, 3],
            vec![1.0.into(), 2.0.into(), 3.0.into()],
            vec![2.0.into(), 3.0.into()], // one short
            vec![0.5.into(), 1.5.into(), 2.5.into()],
            vec![1.0.into(), 2.0.into(), 3.0.into()],
            vec![10.into(), 20.into(), 30.into()],
            vec![1.0.into(), 2.0.into(), 3.0.into()],
        );
        assert_eq!(
            parse_bars(&body).unwrap_err(),
            BarsError::LengthMismatch {
                field: "high",
                got: 2,
                want: 3
            }
        );
    }

    #[test]
    fn a_no_trade_day_is_dropped_not_zero_filled() {
        let n = serde_json::Value::Null;
        let body = ohlcv(
            vec![1, 2, 3],
            vec![1.0.into(), n.clone(), 3.0.into()],
            vec![2.0.into(), n.clone(), 4.0.into()],
            vec![0.5.into(), n.clone(), 2.5.into()],
            vec![1.0.into(), n.clone(), 3.0.into()],
            vec![10.into(), n.clone(), 30.into()],
            vec![1.0.into(), n, 3.0.into()],
        );
        let bars = parse_bars(&body).unwrap();
        assert_eq!(bars.len(), 2, "the null bar is absent, not a row of zeros");
        assert_eq!(bars[1].epoch_seconds, 3);
    }

    #[test]
    fn a_bar_whose_high_is_below_its_low_is_corrupt_not_thin() {
        let body = ohlcv(
            vec![1],
            vec![10.0.into()],
            vec![5.0.into()], // high < low
            vec![9.0.into()],
            vec![10.0.into()],
            vec![100.into()],
            vec![10.0.into()],
        );
        assert_eq!(
            parse_bars(&body).unwrap_err(),
            BarsError::InconsistentBar { index: 0 }
        );
    }

    #[test]
    fn the_default_range_leaves_room_above_the_252_bar_floor() {
        // "1y" comes back at roughly 252 bars, and chan_pos_252 needs 253 —
        // one holiday short of the floor. The default must not be "1y".
        assert_eq!(DEFAULT_BARS_RANGE, "300d");
    }

    #[test]
    fn net_unrealized_trend_right_aligns_and_sums_lots() {
        let a = vec![10.0, 11.0, 12.0, 13.0];
        let b = vec![20.0, 22.0]; // shorter — right-aligned onto last two of a
        let trend = net_unrealized_trend(&[(a, 10.0, 1.0), (b, 20.0, 2.0)]);
        // last two of A: (12-10)*1=2, (13-10)*1=3
        // B: (20-20)*2=0, (22-20)*2=4
        assert_eq!(trend, vec![2.0, 7.0]);
    }

    #[test]
    fn net_unrealized_trend_needs_two_points() {
        assert!(net_unrealized_trend(&[(vec![10.0], 10.0, 1.0)]).is_empty());
        assert!(net_unrealized_trend(&[]).is_empty());
    }
}
