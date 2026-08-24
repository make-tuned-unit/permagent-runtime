//! Overbought sell signals on **open holdings only**.
//!
//! Picker already decided what to rank. This module does not feed those
//! holdings back into the ranker, does not size a position, and cannot place
//! an order. It reads Yahoo daily closes (and the quote's 52-week high) for
//! lots the user already holds and names the overbought signs.
//!
//! Parameters are fixed — the loop must not hunt a threshold on the same
//! data it scores:
//!
//! * RSI-14 vs the user's threshold (default 74)
//! * Stochastic %K-14 ≥ 80
//! * close ≥ 8% above the 20-day SMA
//! * close at or above the 20-day, 2σ upper Bollinger band
//! * close within 2% of the 52-week high
//!
//! A **sell signal** fires if RSI is at/above the threshold, or if two or
//! more of the other four signs are present. One lonely "near the high" is
//! not overbought on its own.

use serde::{Deserialize, Serialize};

use crate::finance_ledger::{self, DEFAULT_RSI_THRESHOLD};
use crate::market_data;
use crate::pick_loop;
use crate::picker;

pub const STOCH_N: usize = 14;
pub const STOCH_OVERBOUGHT: f64 = 80.0;
pub const SMA_N: usize = 20;
/// Close this far above SMA-20 counts as stretched.
pub const SMA_STRETCH: f64 = 0.08;
pub const BB_N: usize = 20;
pub const BB_K: f64 = 2.0;
/// Within this percent of the 52-week high.
pub const NEAR_HIGH_PCT: f64 = 2.0;
const MAX_SYMBOLS: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OverboughtReading {
    pub rsi: Option<f64>,
    pub rsi_threshold: f64,
    pub stochastic_k: Option<f64>,
    pub pct_above_sma20: Option<f64>,
    pub bollinger_pct_b: Option<f64>,
    pub pct_from_52w_high: Option<f64>,
    pub signs: Vec<String>,
    pub signal: bool,
}

impl OverboughtReading {
    /// One-line copy for the tab, the notify path, and the Financier.
    /// Always a signal, never an order.
    pub fn summary(&self, symbol: &str) -> String {
        if !self.signal {
            return format!("No overbought sell signal on {symbol}.");
        }
        let body = if self.signs.is_empty() {
            "overbought signs present".to_string()
        } else {
            self.signs.join("; ")
        };
        format!("Sell signal on {symbol} — {body}. A signal, not an order.")
    }
}

/// Score one ticker. `closes` is oldest → newest. `high_52w` comes from the
/// live quote when we have one; it is not inferred from a shorter window.
pub fn assess(closes: &[f64], high_52w: Option<f64>, rsi_threshold: f64) -> OverboughtReading {
    let rsi = pick_loop::rsi_14(closes);
    let stochastic_k = stochastic_k(closes, STOCH_N);
    let sma = sma(closes, SMA_N);
    let last = closes.last().copied();
    let pct_above_sma20 = match (last, sma) {
        (Some(px), Some(m)) if m > 0.0 => Some(px / m - 1.0),
        _ => None,
    };
    let bb = bollinger(closes, BB_N, BB_K);
    let bollinger_pct_b = match (last, bb) {
        (Some(px), Some((lo, hi))) if (hi - lo).abs() > f64::EPSILON => Some((px - lo) / (hi - lo)),
        _ => None,
    };
    let pct_from_52w_high = match (last, high_52w) {
        (Some(px), Some(hi)) if hi > 0.0 => Some((px / hi - 1.0) * 100.0),
        _ => None,
    };

    let mut signs = Vec::new();
    let mut extra = 0usize;

    let rsi_hot = rsi.map(|v| v >= rsi_threshold).unwrap_or(false);
    if let Some(v) = rsi {
        if rsi_hot {
            signs.push(format!(
                "RSI {:.0} — above your {:.0} threshold",
                v, rsi_threshold
            ));
        }
    }
    if let Some(k) = stochastic_k {
        if k >= STOCH_OVERBOUGHT {
            signs.push(format!(
                "stochastic %K {:.0} — at or above {STOCH_OVERBOUGHT:.0}",
                k
            ));
            extra += 1;
        }
    }
    if let Some(stretch) = pct_above_sma20 {
        if stretch >= SMA_STRETCH {
            signs.push(format!(
                "{:.1}% above the {SMA_N}-day average",
                stretch * 100.0
            ));
            extra += 1;
        }
    }
    if let Some(pct_b) = bollinger_pct_b {
        if pct_b >= 1.0 {
            signs.push("at or above the upper Bollinger band".into());
            extra += 1;
        }
    }
    if let Some(from_high) = pct_from_52w_high {
        if from_high >= -NEAR_HIGH_PCT {
            signs.push(format!("{:.1}% from the 52-week high", from_high.abs()));
            extra += 1;
        }
    }

    OverboughtReading {
        rsi,
        rsi_threshold,
        stochastic_k,
        pct_above_sma20,
        bollinger_pct_b,
        pct_from_52w_high,
        signs,
        signal: rsi_hot || extra >= 2,
    }
}

fn stochastic_k(closes: &[f64], n: usize) -> Option<f64> {
    if closes.len() < n {
        return None;
    }
    let window = &closes[closes.len() - n..];
    let last = *window.last()?;
    let lo = window.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = window.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !(lo.is_finite() && hi.is_finite()) || (hi - lo).abs() < f64::EPSILON {
        return None;
    }
    Some(100.0 * (last - lo) / (hi - lo))
}

fn sma(closes: &[f64], n: usize) -> Option<f64> {
    if closes.len() < n {
        return None;
    }
    let window = &closes[closes.len() - n..];
    Some(window.iter().sum::<f64>() / n as f64)
}

fn bollinger(closes: &[f64], n: usize, k: f64) -> Option<(f64, f64)> {
    let mid = sma(closes, n)?;
    let window = &closes[closes.len() - n..];
    let var = window.iter().map(|x| (x - mid).powi(2)).sum::<f64>() / n as f64;
    let sd = var.sqrt();
    Some((mid - k * sd, mid + k * sd))
}

/// Open lots the user already holds. Union of Picker history and the Finance
/// tab ledger, deduped. Never a pick list, never a watchlist.
pub async fn open_symbols(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    if let Ok(raw) = picker::trades().await {
        for t in raw.iter().filter_map(picker::parse_trade_row) {
            if t.exit_date.is_none() && !out.iter().any(|s| s == &t.ticker) {
                out.push(t.ticker);
            }
        }
    }
    for p in finance_ledger::list_positions(pool).await? {
        if p.exit_date.is_none() && !out.iter().any(|s| s == &p.symbol) {
            out.push(p.symbol);
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct OpenLotReading {
    pub symbol: String,
    pub reading: OverboughtReading,
    pub quote_error: Option<String>,
}

/// Yahoo daily closes + 52-week high for each open lot, capped. Failures stay
/// failures — a missing series is not a silent "not overbought".
pub async fn assess_open_lots(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    rsi_threshold: f64,
) -> Result<Vec<OpenLotReading>, String> {
    let mut symbols = open_symbols(pool).await?;
    symbols.sort();
    symbols.dedup();
    symbols.truncate(MAX_SYMBOLS);
    let mut out = Vec::with_capacity(symbols.len());
    for symbol in symbols {
        let high_52w = match market_data::quote(&symbol).await {
            Ok(q) => q.fifty_two_week_high,
            Err(_) => None,
        };
        match market_data::daily_closes(&symbol, "6mo").await {
            Ok(closes) => out.push(OpenLotReading {
                symbol,
                reading: assess(&closes, high_52w, rsi_threshold),
                quote_error: None,
            }),
            Err(e) => out.push(OpenLotReading {
                symbol,
                reading: OverboughtReading {
                    rsi: None,
                    rsi_threshold,
                    stochastic_k: None,
                    pct_above_sma20: None,
                    bollinger_pct_b: None,
                    pct_from_52w_high: None,
                    signs: vec![],
                    signal: false,
                },
                quote_error: Some(e),
            }),
        }
    }
    Ok(out)
}

/// The Watcher delivers these; the Financier computes them. Daily-per-symbol
/// dedup lives in `finance_rsi_alerts`. A signal is never an order.
pub async fn notify_open_lots(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<Vec<String>, String> {
    let threshold = rsi_threshold();
    let lots = assess_open_lots(pool, threshold).await?;
    let day = chrono::Utc::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let mut sent = Vec::new();
    for lot in lots {
        if !lot.reading.signal {
            continue;
        }
        if crate::finance_ledger::rsi_alert_seen_today(pool, &lot.symbol, &day).await? {
            continue;
        }
        let rsi = lot.reading.rsi.unwrap_or(0.0);
        crate::finance_ledger::record_rsi_alert(pool, &lot.symbol, &day, rsi, threshold).await?;
        let message = lot.reading.summary(&lot.symbol);
        crate::events::emit(crate::events::proactive_nudge(
            "sell_signal",
            &lot.symbol,
            &message,
            1,
            &chrono::Utc::now().to_rfc3339(),
            None,
            None,
        ));
        sent.push(message);
    }
    Ok(sent)
}

pub fn rsi_threshold() -> f64 {
    crate::config::Config::global()
        .get_param::<f64>(crate::finance_ledger::RSI_THRESHOLD_KEY)
        .unwrap_or(DEFAULT_RSI_THRESHOLD)
}

/// The Financier's write on the same key the tab reads. Clamped so a typo
/// cannot silence every signal or fire on every print.
pub fn set_rsi_threshold(value: f64) -> Result<f64, String> {
    if !value.is_finite() || !(50.0..=90.0).contains(&value) {
        return Err("RSI threshold must be a number between 50 and 90".into());
    }
    crate::config::Config::global()
        .set_param(crate::finance_ledger::RSI_THRESHOLD_KEY, value)
        .map_err(|e| e.to_string())?;
    Ok(value)
}

/// Copy the Financier reads aloud. Distinguishes "could not ask" from "asked
/// and there was no signal".
pub fn describe_open_lots(lots: &[OpenLotReading]) -> String {
    if lots.is_empty() {
        return "No open positions to check for overbought sell signals.".into();
    }
    let mut sections = Vec::new();
    sections.push(format!(
        "{} open lot(s). Overbought sell signals use Yahoo daily closes on holdings you already have. Holdings never go into the Picker ranker. A signal is not an order and is not a position size.",
        lots.len()
    ));
    for lot in lots {
        if let Some(err) = &lot.quote_error {
            sections.push(format!(
                "{} — could not fetch daily closes ({err}). Do not invent a signal.",
                lot.symbol
            ));
            continue;
        }
        let r = &lot.reading;
        if r.signal {
            sections.push(r.summary(&lot.symbol));
        } else {
            let rsi = r
                .rsi
                .map(|v| format!("RSI {v:.0}"))
                .unwrap_or_else(|| "RSI unavailable".into());
            sections.push(format!(
                "{} — no overbought sell signal ({rsi}, threshold {:.0}).",
                lot.symbol, r.rsi_threshold
            ));
        }
    }
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn climb(n: usize) -> Vec<f64> {
        (0..n).map(|i| 50.0 + i as f64).collect()
    }

    #[test]
    fn a_straight_climb_is_a_sell_signal() {
        let r = assess(&climb(40), Some(90.0), 74.0);
        assert!(r.signal, "climb should signal, reading={r:?}");
        assert!(r.rsi.unwrap() > 70.0);
        assert!(r.summary("SHOP").starts_with("Sell signal on SHOP"));
        assert!(r.summary("SHOP").contains("not an order"));
    }

    #[tokio::test]
    async fn no_open_lots_means_no_alerts() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::apply_finance_ledger_schema(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::apply_finance_spend_schema(&pool)
            .await
            .unwrap();
        let sent = notify_open_lots(&pool).await.unwrap();
        assert!(sent.is_empty());
    }

    #[test]
    fn a_choppy_series_is_not_a_sell_signal() {
        let closes: Vec<f64> = (0..40)
            .map(|i| 100.0 + if i % 2 == 0 { 1.5 } else { -1.2 })
            .collect();
        let r = assess(&closes, Some(200.0), 74.0);
        assert!(!r.signal, "chop should not signal, reading={r:?}");
        assert!(r.summary("ENB").starts_with("No overbought"));
    }

    #[test]
    fn two_non_rsi_signs_still_fire_when_rsi_bar_is_high() {
        // Climb is at the window high (stoch ~100) and at/above the upper band.
        // RSI threshold 99 so RSI alone would not fire.
        let r = assess(&climb(40), Some(89.5), 99.0);
        assert!(
            r.signal,
            "stoch + band (and near-high) must fire without RSI, reading={r:?}"
        );
        assert!(
            r.signs.iter().any(|s| s.contains("stochastic")),
            "{:?}",
            r.signs
        );
    }

    #[test]
    fn near_the_high_alone_is_not_a_signal() {
        // Alternating chop so RSI/stoch/bands stay mid; last print equals the
        // quoted 52-week high — one sign, not a signal.
        let closes: Vec<f64> = (0..40)
            .map(|i| 100.0 + if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let last = *closes.last().unwrap();
        let r = assess(&closes, Some(last), 74.0);
        assert!(!r.signal, "near-high alone must not fire, reading={r:?}");
    }

    #[test]
    fn short_history_does_not_invent_numbers() {
        let r = assess(&[10.0, 11.0], None, 74.0);
        assert!(r.rsi.is_none());
        assert!(!r.signal);
    }

    #[test]
    fn rsi_threshold_write_refuses_a_typo() {
        assert!(set_rsi_threshold(f64::NAN).is_err());
        assert!(set_rsi_threshold(12.0).is_err());
        assert!(set_rsi_threshold(99.0).is_err());
    }
}
