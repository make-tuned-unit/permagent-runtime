//! Market data — live quotes and fundamentals, with no setup required.
//!
//! The Financier's research half has to work for a user who has no trading
//! stack of their own, so this reaches Yahoo Finance's public quote endpoints
//! directly: no API key, no account, no local service.
//!
//! ## What that costs, stated plainly
//!
//! These endpoints are **not a supported API**. They are the ones Yahoo's own
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
//! This is deliberately a *research* source, not an execution source. Nothing
//! in this module can place an order.

use serde::{Deserialize, Serialize};
use std::time::Duration;

const QUOTE_BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const TIMEOUT: Duration = Duration::from_secs(12);

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

/// Fetch one symbol. `Err` means we could not get an answer — the caller must
/// say so rather than substituting anything it remembers.
pub async fn quote(symbol: &str) -> Result<Quote, String> {
    let symbol = normalize_symbol(symbol)?;
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{QUOTE_BASE}/{symbol}?interval=1d&range=5d");
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
    parse_quote(&body, &symbol)
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
}
