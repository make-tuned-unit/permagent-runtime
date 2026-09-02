//! Exit signals on **open holdings only**, and the one path they leave by.
//!
//! Picker already decided what to rank. This module does not feed those
//! holdings back into the ranker, does not size a position, and **cannot place
//! an order**. It reads adjusted daily OHLCV for lots the user already holds,
//! names what fired, and files a proposal.
//!
//! ## Two families of signal, one exit
//!
//! *Overbought* — the original set, and a reason to consider taking a gain:
//!
//! * RSI-14 vs the user's threshold (default 74)
//! * Stochastic %K-14 ≥ 80
//! * close ≥ 8% above the 20-day SMA
//! * close at or above the 20-day, 2σ upper Bollinger band
//! * close within 2% of the 52-week high
//!
//! A signal fires if RSI is at/above the threshold, or if two or more of the
//! other four signs are present. One lonely "near the high" is not overbought.
//!
//! *Breakdown* — F0 §5.3, and a reason to consider cutting a loss: a 20-day
//! channel break CONJOINED with a close below the 100-day average and held for
//! two consecutive closes; a 3×ATR20 chandelier stop measured from the highest
//! close since entry; and, escalated, a 55-day channel break or a 25% drawdown
//! from the entry price.
//!
//! Those two families used to have two different destinations — the first a
//! toast, the second nothing at all. They now share one: a typed proposal in
//! the Decision Inbox, deduped per holding per rule. A toast is a thing you
//! miss; a proposal waits.
//!
//! ## The asymmetry is deliberate
//!
//! The bar to OPEN a position is strictly higher than the bar to KEEP one —
//! Novy-Marx & Velikov's buy/hold spread. The buy side wants a fresh 55-day
//! breakout with volume confirmation; a 20-day breakdown plus a trend break is
//! enough to raise a notice on a name already held. Do not symmetrise these
//! for elegance.
//!
//! ## Volume triggers nothing here
//!
//! F0 §3.3 searched for evidence behind volume-divergence exits and found none
//! at any usable grade. `rvol` appears in a notice as labelled context and is
//! not part of any trigger.
//!
//! ## It proposes; it never trades
//!
//! Nothing in the decision effect-apply path touches Picker's journal or any
//! brokerage, for this kind or any other. An approve records that the user
//! agreed with the rule. The selling, if any, is theirs.

use serde::{Deserialize, Serialize};

use crate::decisions::{self, SellNoticeEvidence, SellNoticePayload};
use crate::finance_indicators as ind;
use crate::finance_ledger::{self, DEFAULT_RSI_THRESHOLD};
use crate::market_data::{self, DailyBar};
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

/// F0 §5.3 escalation: a quarter of the entry price gone.
pub const DRAWDOWN_URGENT: f64 = 0.25;

/// Stable dedupe keys, one per rule. They are the axis
/// [`crate::decisions::find_open_sell_notice`] keys on, so renaming one
/// silently re-files every open notice of that kind. They are constants, and
/// they are not display strings.
pub const TRIGGER_DONCHIAN20_SMA100: &str = "donchian20_below_sma100";
pub const TRIGGER_CHANDELIER: &str = "chandelier_3atr";
pub const TRIGGER_DONCHIAN55: &str = "donchian55_breakdown";
pub const TRIGGER_DRAWDOWN: &str = "drawdown_from_entry";
pub const TRIGGER_OVERBOUGHT: &str = "overbought_composite";

/// One rule that fired on one holding.
///
/// `rule_fired` carries **both** numbers of its own inequality. A rule stated
/// without its numbers ("the 20-day channel broke") cannot be checked by the
/// person reading it, and cannot be checked later against the bar that
/// triggered it either.
#[derive(Debug, Clone, PartialEq)]
pub struct ExitSignal {
    pub trigger: &'static str,
    /// Escalates the proposal to Tier 2, user-only.
    pub urgent: bool,
    pub rule_fired: String,
    /// The price at which this signal would be void.
    pub invalidation_level: Option<f64>,
    pub invalidation_note: String,
}

/// One holding, as the exit engine needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenLot {
    pub symbol: String,
    pub entry_date: String,
    pub entry_price: f64,
    pub shares: i64,
}

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

// ---------------------------------------------------------------------------
// Breakdown exits (F0 §5.3)
// ---------------------------------------------------------------------------

/// Does the (i) condition hold on this series' final bar?
/// `close < dch_lo(20) AND close < sma(100)` — the channel break CONJOINED
/// with the trend break, never either alone.
fn breaks_channel_and_trend(bars: &[DailyBar]) -> Option<(f64, f64, f64)> {
    let close = bars.last()?.close;
    let lo = ind::donchian_lo(bars, ind::DONCHIAN_SHORT).ok()?;
    let sma100 = ind::sma(bars, ind::SMA_MID).ok()?;
    (close < lo && close < sma100).then_some((close, lo, sma100))
}

/// Where in `bars` the position was opened, and whether the history reaches
/// back that far.
///
/// `truncated` matters: the chandelier hangs off the highest close SINCE
/// ENTRY, so a lot opened before the fetched window gets a high-water mark
/// measured from the window's start instead. That makes the stop no higher
/// than the truth and possibly lower, which is the safe direction — but it is
/// still a different number from the one the rule names, so the notice says so
/// rather than quietly using it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntryAnchor {
    pub index: usize,
    pub truncated: bool,
}

/// Locate the entry bar by date. `entry_date` is `YYYY-MM-DD`.
pub fn entry_anchor(bars: &[DailyBar], entry_date: &str) -> Option<EntryAnchor> {
    let date = chrono::NaiveDate::parse_from_str(entry_date.trim(), "%Y-%m-%d").ok()?;
    let epoch = date.and_hms_opt(0, 0, 0)?.and_utc().timestamp();
    let index = bars.iter().position(|b| b.epoch_seconds >= epoch)?;
    // Index 0 is ambiguous — it is both "opened on the oldest bar we have" and
    // "opened before the window began". The timestamp settles it.
    let truncated = index == 0 && bars[0].epoch_seconds > epoch + 86_400;
    Some(EntryAnchor { index, truncated })
}

/// F0 §5.3's WARN tier: logged, never filed. A single 20-day break, or a close
/// below the 50-day average, is common enough in these names that filing on it
/// would turn the inbox into a ticker tape.
pub fn exit_warnings(bars: &[DailyBar]) -> Vec<String> {
    let Some(close) = bars.last().map(|b| b.close) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(sma50) = ind::sma(bars, ind::SMA_FAST) {
        if close < sma50 {
            out.push(format!("close {close:.2} < sma(50) {sma50:.2}"));
        }
    }
    if let Ok(lo) = ind::donchian_lo(bars, ind::DONCHIAN_SHORT) {
        if close < lo {
            out.push(format!("close {close:.2} < dch_lo(20) {lo:.2}"));
        }
    }
    out
}

/// Every exit rule that fired on this holding, in escalation order.
///
/// Pure and deterministic over the bars: no clock, no database, no network.
/// The two-consecutive-closes condition on rule (i) is evaluated by re-running
/// the rule against yesterday's series rather than by remembering yesterday —
/// state that lives in the price history cannot go stale, be lost on restart,
/// or disagree with the chart.
pub fn assess_exits(bars: &[DailyBar], lot: Option<&OpenLot>) -> Vec<ExitSignal> {
    let Some(close) = bars.last().map(|b| b.close) else {
        return Vec::new();
    };
    let mut out = Vec::new();

    // ── URGENT: a 55-day low ──
    if let Ok(lo55) = ind::donchian_lo(bars, ind::DONCHIAN_LONG) {
        if close < lo55 {
            out.push(ExitSignal {
                trigger: TRIGGER_DONCHIAN55,
                urgent: true,
                rule_fired: format!("close {close:.2} < dch_lo(55) {lo55:.2}"),
                invalidation_level: Some(lo55),
                invalidation_note: format!(
                    "a close back above the 55-day low of {lo55:.2} voids this signal"
                ),
            });
        }
    }

    // ── URGENT: a quarter of the entry gone ──
    if let Some(l) = lot {
        if l.entry_price > 0.0 {
            let drawdown = (l.entry_price - close) / l.entry_price;
            if drawdown > DRAWDOWN_URGENT {
                let level = l.entry_price * (1.0 - DRAWDOWN_URGENT);
                out.push(ExitSignal {
                    trigger: TRIGGER_DRAWDOWN,
                    urgent: true,
                    rule_fired: format!(
                        "close {close:.2} is {:.1}% below the {:.2} entry, past the {:.0}% \
                         escalation",
                        drawdown * 100.0,
                        l.entry_price,
                        DRAWDOWN_URGENT * 100.0
                    ),
                    invalidation_level: Some(level),
                    invalidation_note: format!(
                        "a close back above {level:.2} puts the position inside the {:.0}% band \
                         again",
                        DRAWDOWN_URGENT * 100.0
                    ),
                });
            }
        }
    }

    // ── (i) the 20-day break conjoined with the trend break, held two closes ──
    if bars.len() > 1 {
        if let (Some((c, lo, sma100)), Some(_)) = (
            breaks_channel_and_trend(bars),
            breaks_channel_and_trend(&bars[..bars.len() - 1]),
        ) {
            let invalidation = ind::donchian_hi(bars, ind::DONCHIAN_SHORT).ok();
            out.push(ExitSignal {
                trigger: TRIGGER_DONCHIAN20_SMA100,
                urgent: false,
                rule_fired: format!(
                    "close {c:.2} < dch_lo(20) {lo:.2} AND close {c:.2} < sma(100) {sma100:.2}, \
                     2nd consecutive close"
                ),
                invalidation_level: invalidation,
                invalidation_note: match invalidation {
                    Some(hi) => {
                        format!("a close back above the 20-day high of {hi:.2} voids this signal")
                    }
                    None => "the 20-day high could not be computed from this history".into(),
                },
            });
        }
    }

    // ── (ii) the chandelier, on one close ──
    if let Some(anchor) = lot.and_then(|l| entry_anchor(bars, &l.entry_date)) {
        if let (Ok(stop), Ok(atr)) = (
            ind::chandelier_stop(bars, anchor.index),
            ind::atr(bars, ind::ATR_PERIOD),
        ) {
            if close < stop {
                let peak = bars[anchor.index..]
                    .iter()
                    .fold(f64::MIN, |m, b| m.max(b.close));
                let truncation = if anchor.truncated {
                    " (high-water mark measured from the start of the fetched window, not from \
                     the entry date — the history does not reach that far back)"
                } else {
                    ""
                };
                out.push(ExitSignal {
                    trigger: TRIGGER_CHANDELIER,
                    urgent: false,
                    rule_fired: format!(
                        "close {close:.2} < chandelier {stop:.2} (highest close since entry \
                         {peak:.2} - {:.1} x atr20 {atr:.2}){truncation}",
                        ind::CHANDELIER_K
                    ),
                    invalidation_level: Some(stop),
                    invalidation_note: format!(
                        "a close back above {stop:.2} voids this signal; the level ratchets up \
                         with the high-water mark and never down"
                    ),
                });
            }
        }
    }

    out
}

/// The overbought composite, expressed as an exit signal so it travels the
/// same road as every other rule.
///
/// It used to end in a toast and nothing else. The signal has not changed; its
/// destination has.
pub fn overbought_exit(reading: &OverboughtReading, closes: &[f64]) -> Option<ExitSignal> {
    if !reading.signal {
        return None;
    }
    let sma20 = sma(closes, SMA_N);
    Some(ExitSignal {
        trigger: TRIGGER_OVERBOUGHT,
        urgent: false,
        rule_fired: reading.signs.join("; "),
        invalidation_level: sma20,
        invalidation_note: match sma20 {
            Some(m) => format!(
                "the reading is void once the close is back at or below the {SMA_N}-day average \
                 of {m:.2}"
            ),
            None => format!("the {SMA_N}-day average could not be computed from this history"),
        },
    })
}

// ---------------------------------------------------------------------------
// The one path out
// ---------------------------------------------------------------------------

/// Open lots the user already holds. Union of Picker's journal and the Finance
/// tab ledger, deduped by symbol. Never a pick list, never a watchlist.
pub async fn open_lots(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<Vec<OpenLot>, String> {
    let mut out: Vec<OpenLot> = Vec::new();
    if let Ok(raw) = picker::trades().await {
        for t in raw.iter().filter_map(picker::parse_trade_row) {
            if t.exit_date.is_none() && !out.iter().any(|l| l.symbol == t.ticker) {
                out.push(OpenLot {
                    symbol: t.ticker,
                    entry_date: t.entry_date,
                    entry_price: t.entry_price,
                    shares: t.shares,
                });
            }
        }
    }
    for p in finance_ledger::list_positions(pool).await? {
        if p.exit_date.is_none() && !out.iter().any(|l| l.symbol == p.symbol) {
            out.push(OpenLot {
                symbol: p.symbol,
                entry_date: p.entry_date,
                entry_price: p.entry_price,
                shares: p.shares,
            });
        }
    }
    out.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    out.truncate(MAX_SYMBOLS);
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct OpenLotReading {
    pub lot: OpenLot,
    pub reading: OverboughtReading,
    /// Every rule that fired, overbought composite included.
    pub exits: Vec<ExitSignal>,
    /// F0's WARN tier — said aloud, never filed.
    pub warnings: Vec<String>,
    /// The bars the reads were computed from; kept so the filer does not refetch.
    pub bars: Vec<DailyBar>,
    pub quote_error: Option<String>,
}

impl OpenLotReading {
    pub fn symbol(&self) -> &str {
        &self.lot.symbol
    }
}

/// Adjusted daily OHLCV + the quote's 52-week high for each open lot, capped.
/// Failures stay failures — a missing series is not a silent "no signal".
///
/// The window widened from `6mo` to 300 bars when the breakdown rules arrived:
/// `sma(100)` needs 100 and the 55-day channel needs 56, which 6mo could just
/// about carry, but `chan_pos_252` in the notice's evidence needs 253 and could
/// not. One fetch, one window, every rule on the same bars.
pub async fn assess_open_lots(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    rsi_threshold: f64,
) -> Result<Vec<OpenLotReading>, String> {
    let lots = open_lots(pool).await?;
    let mut out = Vec::with_capacity(lots.len());
    for lot in lots {
        let high_52w = match market_data::quote(&lot.symbol).await {
            Ok(q) => q.fifty_two_week_high,
            Err(_) => None,
        };
        match market_data::daily_bars(&lot.symbol, market_data::DEFAULT_BARS_RANGE).await {
            Ok(bars) => {
                let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
                let reading = assess(&closes, high_52w, rsi_threshold);
                let mut exits = assess_exits(&bars, Some(&lot));
                exits.extend(overbought_exit(&reading, &closes));
                out.push(OpenLotReading {
                    lot,
                    reading,
                    exits,
                    warnings: exit_warnings(&bars),
                    bars,
                    quote_error: None,
                });
            }
            Err(e) => out.push(OpenLotReading {
                lot,
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
                exits: Vec::new(),
                warnings: Vec::new(),
                bars: Vec::new(),
                quote_error: Some(e.to_string()),
            }),
        }
    }
    Ok(out)
}

/// The plain-language headline for a card, ≤ 80 chars by construction.
fn headline_for(trigger: &str, symbol: &str) -> String {
    let text = match trigger {
        TRIGGER_DONCHIAN55 => format!("{symbol} has broken to a new 55-day low"),
        TRIGGER_DRAWDOWN => format!("{symbol} is down more than a quarter from your entry"),
        TRIGGER_DONCHIAN20_SMA100 => format!("{symbol} has broken below its range and its trend"),
        TRIGGER_CHANDELIER => format!("{symbol} has fallen past its trailing stop level"),
        TRIGGER_OVERBOUGHT => format!("{symbol} looks stretched — consider taking the gain"),
        _ => format!("{symbol} raised an exit signal worth a look"),
    };
    decisions::truncate_for_headline(&text)
}

/// Build the notice payload. Pure, so the exact card can be asserted in a test
/// rather than described in a comment.
pub fn sell_notice_payload(
    lot: &OpenLot,
    signal: &ExitSignal,
    bars: &[DailyBar],
    trigger_date: &str,
) -> SellNoticePayload {
    let close = bars.last().map(|b| b.close).unwrap_or(0.0);
    let trend_ok = ind::trend_ok(bars).ok();
    let chan_pos = ind::chan_pos_252(bars).ok();
    let reversal = ind::reversal_risk(bars).ok();

    // Every notice carries its own counter-case. Without one the reader only
    // ever sees the argument for selling, which is advocacy, not evidence.
    let mut counter_evidence = vec![match trend_ok {
        Some(true) => "trend_ok is still TRUE — the close is above its 200-day average and the \
             50-day is above the 200-day, so the long trend has not broken"
            .to_string(),
        Some(false) => "trend_ok is false — the long trend is already broken, so this is not a \
             single bad week"
            .to_string(),
        None => "trend_ok could not be computed from the history available".to_string(),
    }];
    counter_evidence.push(match chan_pos {
        Some(v) if v >= 0.60 => format!(
            "chan_pos_252 is {v:.2} — still in the upper part of its 52-week range despite this \
             trigger"
        ),
        Some(v) => format!("chan_pos_252 is {v:.2}, in the lower part of its 52-week range"),
        None => "chan_pos_252 could not be computed — fewer than 253 bars of history".to_string(),
    });
    counter_evidence.push(match reversal {
        Some(true) => "reversal_risk is TRUE: turnover has been sustained above twice its \
             long-run level, the regime where reversals are largest in both directions"
            .to_string(),
        Some(false) => "reversal_risk is false — turnover is not elevated".to_string(),
        None => "reversal_risk could not be computed from the history available".to_string(),
    });
    counter_evidence.push(
        "earnings proximity is UNKNOWN — this engine has no earnings calendar, so an earnings \
         date near this trigger cannot be ruled out"
            .to_string(),
    );

    let unrealized_pnl_pct = if lot.entry_price > 0.0 {
        (close / lot.entry_price - 1.0) * 100.0
    } else {
        0.0
    };
    let distance = signal
        .invalidation_level
        .and_then(|level| (close > 0.0).then_some((level / close - 1.0) * 100.0));
    let days_held = chrono::NaiveDate::parse_from_str(lot.entry_date.trim(), "%Y-%m-%d")
        .ok()
        .zip(chrono::NaiveDate::parse_from_str(trigger_date, "%Y-%m-%d").ok())
        .map(|(entry, now)| (now - entry).num_days());

    SellNoticePayload {
        symbol: lot.symbol.clone(),
        trigger: signal.trigger.to_string(),
        urgency: if signal.urgent { "urgent" } else { "advisory" }.to_string(),
        trigger_date: trigger_date.to_string(),
        trigger_close: close,
        rule_fired: signal.rule_fired.clone(),
        invalidation_level: signal.invalidation_level,
        invalidation_note: signal.invalidation_note.clone(),
        distance_to_invalidation_pct: distance,
        entry_date: lot.entry_date.clone(),
        entry_price: lot.entry_price,
        shares: lot.shares,
        unrealized_pnl_pct,
        unrealized_pnl_usd: (close - lot.entry_price) * lot.shares as f64,
        days_held,
        evidence: SellNoticeEvidence {
            dch_lo20: ind::donchian_lo(bars, ind::DONCHIAN_SHORT).ok(),
            dch_hi20: ind::donchian_hi(bars, ind::DONCHIAN_SHORT).ok(),
            dch_lo55: ind::donchian_lo(bars, ind::DONCHIAN_LONG).ok(),
            sma50: ind::sma(bars, ind::SMA_FAST).ok(),
            sma100: ind::sma(bars, ind::SMA_MID).ok(),
            sma200: ind::sma(bars, ind::SMA_SLOW).ok(),
            atr20: ind::atr(bars, ind::ATR_PERIOD).ok(),
            atr_pct: ind::atr_pct(bars).ok(),
            chandelier: entry_anchor(bars, &lot.entry_date)
                .and_then(|a| ind::chandelier_stop(bars, a.index).ok()),
            chan_pos_252: chan_pos,
            rvol: ind::rvol(bars).ok(),
            dollar_volume_20d: ind::dollar_volume_20d_median(bars).ok(),
        },
        counter_evidence,
        caveat: decisions::SELL_NOTICE_CAVEAT.to_string(),
    }
}

/// File ONE exit notice, or decline to.
///
/// The whole dedupe lives here, so every cadence obeys it by construction
/// rather than by remembering to. `Ok(None)` means a notice for this
/// ticker+rule is already open and nothing was filed.
///
/// Separated from [`file_sell_notices`] because that function's only other job
/// is fetching bars over the network — and a rule about when a card may be
/// raised should be testable without one.
pub async fn file_one_notice(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    lot: &OpenLot,
    signal: &ExitSignal,
    bars: &[DailyBar],
    reading: &OverboughtReading,
    day: &str,
) -> Result<Option<String>, String> {
    // The overbought composite keeps its original once-a-day promise on top of
    // the open-notice check: it is the noisiest of the rules and the only one
    // that fires on a name doing WELL, so re-raising it the same afternoon
    // after the morning's card was answered would nag.
    if signal.trigger == TRIGGER_OVERBOUGHT
        && finance_ledger::rsi_alert_seen_today(pool, &lot.symbol, day).await?
    {
        return Ok(None);
    }
    if decisions::find_open_sell_notice(pool, &lot.symbol, signal.trigger)
        .await?
        .is_some()
    {
        return Ok(None);
    }
    let payload = sell_notice_payload(lot, signal, bars, day);
    let action_class = if signal.urgent {
        decisions::SELL_NOTICE_URGENT
    } else {
        decisions::SELL_NOTICE_ADVISORY
    };
    let detail = format!(
        "{}\n\nInvalidation: {}\n\nPosition: {} shares from {} at {:.2}; unrealized {:+.1}%.\n\n\
         Counter-evidence:\n{}\n\n{}",
        payload.rule_fired,
        payload.invalidation_note,
        payload.shares,
        payload.entry_date,
        payload.entry_price,
        payload.unrealized_pnl_pct,
        payload.counter_evidence.join("\n"),
        payload.caveat,
    );
    let request = decisions::NewDecision {
        kind: "risk_gate".to_string(),
        headline: Some(headline_for(signal.trigger, &lot.symbol)),
        detail: Some(detail),
        payload: serde_json::to_value(decisions::RiskGatePayload {
            action_class: action_class.to_string(),
            description: payload.rule_fired.clone(),
            requested_by: "financier".to_string(),
            repo_target: None,
            sell_notice: Some(payload.clone()),
        })
        .map_err(|e| e.to_string())?,
        action_class: Some(action_class.to_string()),
        ..Default::default()
    };
    let decision = decisions::create_decision(pool, request).await?;
    if decision.kind == "malformed" {
        return Err(format!(
            "the sell notice for {} failed its own payload schema: {}",
            lot.symbol, decision.detail
        ));
    }
    if signal.trigger == TRIGGER_OVERBOUGHT {
        finance_ledger::record_rsi_alert(
            pool,
            &lot.symbol,
            day,
            reading.rsi.unwrap_or(0.0),
            reading.rsi_threshold,
        )
        .await?;
    }
    Ok(Some(format!(
        "{} — {} ({}). {}",
        lot.symbol, payload.rule_fired, payload.urgency, payload.invalidation_note
    )))
}

/// File the day's exit notices. **The** path — there is no other.
///
/// Called by every cadence. The 15:30 close scan runs first on a trading day;
/// the 6-hour sweep calls the same function and, finding an open notice for
/// every ticker+rule the close scan already raised, files nothing. That is the
/// deferral: not a special case in the sweep, but the same dedupe both sides
/// obey.
///
/// Returns one line per notice actually filed, for the caller's log.
pub async fn file_sell_notices(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<Vec<String>, String> {
    let threshold = rsi_threshold();
    let lots = assess_open_lots(pool, threshold).await?;
    let day = chrono::Utc::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let mut filed = Vec::new();
    for lot in lots {
        for warning in &lot.warnings {
            tracing::debug!(
                target: "permagent::finance",
                "{} warn (not filed): {warning}", lot.symbol()
            );
        }
        for signal in &lot.exits {
            if let Some(line) =
                file_one_notice(pool, &lot.lot, signal, &lot.bars, &lot.reading, &day).await?
            {
                filed.push(line);
            }
        }
    }
    Ok(filed)
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
        return "No open positions to check for sell signals.".into();
    }
    let mut sections = Vec::new();
    sections.push(format!(
        "{} open lot(s). Exit signals use adjusted daily bars on holdings you already have — the \
         overbought set (RSI, stochastic, Bollinger, 52-week) and the breakdown set (20-day \
         channel with the 100-day average, the 3xATR chandelier, and the escalated 55-day \
         channel and 25% drawdown). Holdings never go into the Picker ranker. Everything here is \
         a proposal; nothing in Permagent can place a trade.",
        lots.len()
    ));
    for lot in lots {
        if let Some(err) = &lot.quote_error {
            sections.push(format!(
                "{} — could not fetch daily bars ({err}). Do not invent a signal.",
                lot.symbol()
            ));
            continue;
        }
        for signal in &lot.exits {
            sections.push(format!(
                "{} — {}{}. {} A signal, not an order.",
                lot.symbol(),
                signal.rule_fired,
                if signal.urgent { " [URGENT]" } else { "" },
                signal.invalidation_note,
            ));
        }
        if lot.exits.is_empty() {
            let rsi = lot
                .reading
                .rsi
                .map(|v| format!("RSI {v:.0}"))
                .unwrap_or_else(|| "RSI unavailable".into());
            sections.push(format!(
                "{} — no exit signal ({rsi}, threshold {:.0}).",
                lot.symbol(),
                lot.reading.rsi_threshold
            ));
        }
        for warning in &lot.warnings {
            sections.push(format!(
                "{} — warning only, not filed: {warning}.",
                lot.symbol()
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
    async fn no_open_lots_means_no_notices() {
        let pool = test_pool().await;
        let filed = file_sell_notices(&pool).await.unwrap();
        assert!(filed.is_empty());
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

    // ── fixtures ──────────────────────────────────────────────────────────

    async fn test_pool() -> sqlx::Pool<sqlx::Sqlite> {
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
        pool
    }

    /// Bar `i` of a series starting 2026-01-01, timestamped at the exchange
    /// open so `entry_anchor`'s midnight comparison is exercised the way the
    /// real Yahoo payload exercises it.
    fn day_epoch(i: usize) -> i64 {
        chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .checked_add_days(chrono::Days::new(i as u64))
            .unwrap()
            .and_hms_opt(14, 30, 0)
            .unwrap()
            .and_utc()
            .timestamp()
    }

    fn day_str(i: usize) -> String {
        chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .checked_add_days(chrono::Days::new(i as u64))
            .unwrap()
            .format("%Y-%m-%d")
            .to_string()
    }

    fn b(i: usize, high: f64, low: f64, close: f64) -> DailyBar {
        DailyBar {
            epoch_seconds: day_epoch(i),
            open: close,
            high,
            low,
            close,
            volume: 1_000,
        }
    }

    /// A close with the high/low straddling it by 1.
    fn c(i: usize, close: f64) -> DailyBar {
        b(i, close + 1.0, close - 1.0, close)
    }

    fn lot(entry_price: f64, entry_day: usize) -> OpenLot {
        OpenLot {
            symbol: "TEST".into(),
            entry_date: day_str(entry_day),
            entry_price,
            shares: 100,
        }
    }

    /// The (i) fixture: 100 flat sessions at 100, a 40-session shelf at 110,
    /// then two closes that break BOTH the 20-day channel and the 100-day
    /// average — while staying above the 55-day low, so the urgent rule is not
    /// what is being measured.
    fn channel_and_trend_break() -> Vec<DailyBar> {
        let mut bars: Vec<DailyBar> = (0..100).map(|i| c(i, 100.0)).collect();
        bars.extend((100..140).map(|i| c(i, 110.0)));
        // Bar 140's low sits AT its close so tomorrow's 20-day channel is not
        // dragged down by today's own break.
        bars.push(b(140, 101.0, 100.0, 100.0));
        bars.push(b(141, 100.0, 98.0, 99.0));
        bars
    }

    // ── the breakdown rules ───────────────────────────────────────────────

    #[test]
    fn the_channel_break_and_the_trend_break_must_both_hold_for_two_closes() {
        let bars = channel_and_trend_break();
        let signals = assess_exits(&bars, None);
        let s = signals
            .iter()
            .find(|s| s.trigger == TRIGGER_DONCHIAN20_SMA100)
            .unwrap_or_else(|| panic!("no channel+trend signal in {signals:#?}"));
        assert!(!s.urgent, "the 20-day break is the advisory tier");
        // Both numbers of both inequalities are in the rule, substituted.
        assert!(
            s.rule_fired.contains("close 99.00 < dch_lo(20) 100.00"),
            "{}",
            s.rule_fired
        );
        assert!(s.rule_fired.contains("sma(100)"), "{}", s.rule_fired);
        assert!(
            s.rule_fired.contains("2nd consecutive close"),
            "{}",
            s.rule_fired
        );
        // Invalidation is the 20-day HIGH — reclaiming the channel voids it.
        assert_eq!(
            s.invalidation_level,
            Some(crate::finance_indicators::donchian_hi(&bars, 20).unwrap())
        );
    }

    #[test]
    fn one_close_below_the_channel_is_a_warning_not_a_notice() {
        let bars = channel_and_trend_break();
        let one_day_earlier = &bars[..bars.len() - 1];
        let signals = assess_exits(one_day_earlier, None);
        assert!(
            !signals
                .iter()
                .any(|s| s.trigger == TRIGGER_DONCHIAN20_SMA100),
            "a single close must not file: {signals:#?}"
        );
        // It is not silence, though — the warning tier says it out loud.
        let warnings = exit_warnings(one_day_earlier);
        assert!(
            warnings.iter().any(|w| w.contains("dch_lo(20)")),
            "{warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("sma(50)")),
            "{warnings:?}"
        );
    }

    #[test]
    fn the_channel_break_alone_without_the_trend_break_files_nothing() {
        // Two closes under the 20-day low, but comfortably ABOVE the 100-day
        // average — a pullback inside an uptrend, which F0 §5.3 conjoins
        // precisely so it is not treated as a breakdown.
        let mut bars: Vec<DailyBar> = (0..100).map(|i| c(i, 100.0)).collect();
        bars.extend((100..140).map(|i| c(i, 100.0 + (i - 99) as f64 * 2.0)));
        bars.push(b(140, 141.0, 140.0, 140.0));
        bars.push(b(141, 140.0, 138.0, 139.0));
        let close = bars.last().unwrap().close;
        let sma100 = crate::finance_indicators::sma(&bars, 100).unwrap();
        assert!(close > sma100, "the fixture must still be in its uptrend");
        assert!(
            close < crate::finance_indicators::donchian_lo(&bars, 20).unwrap(),
            "…while under its 20-day channel"
        );
        assert!(!assess_exits(&bars, None)
            .iter()
            .any(|s| s.trigger == TRIGGER_DONCHIAN20_SMA100));
    }

    #[test]
    fn a_fifty_five_day_low_is_urgent() {
        let mut bars: Vec<DailyBar> = (0..100).map(|i| c(i, 100.0)).collect();
        bars.push(b(100, 91.0, 89.0, 90.0));
        let s = assess_exits(&bars, None)
            .into_iter()
            .find(|s| s.trigger == TRIGGER_DONCHIAN55)
            .expect("a new 55-day low must fire");
        assert!(s.urgent, "F0 §5.3 escalates the 55-day break");
        assert!(
            s.rule_fired.contains("close 90.00 < dch_lo(55) 99.00"),
            "{}",
            s.rule_fired
        );
        assert_eq!(s.invalidation_level, Some(99.0));
    }

    #[test]
    fn a_quarter_lost_from_the_entry_is_urgent() {
        let mut bars: Vec<DailyBar> = (0..100).map(|i| c(i, 100.0)).collect();
        bars.push(c(100, 70.0));
        let held = lot(100.0, 5);
        let s = assess_exits(&bars, Some(&held))
            .into_iter()
            .find(|s| s.trigger == TRIGGER_DRAWDOWN)
            .expect("a 30% drawdown must fire");
        assert!(s.urgent);
        assert!(s.rule_fired.contains("30.0% below"), "{}", s.rule_fired);
        assert_eq!(s.invalidation_level, Some(75.0));

        // 24% does not. The threshold is a threshold.
        let mut shallow: Vec<DailyBar> = (0..100).map(|i| c(i, 100.0)).collect();
        shallow.push(c(100, 76.0));
        assert!(!assess_exits(&shallow, Some(&held))
            .iter()
            .any(|s| s.trigger == TRIGGER_DRAWDOWN));
    }

    #[test]
    fn the_chandelier_hangs_off_the_high_water_close_since_entry() {
        // 21 flat bars set ATR20 = 2.0 exactly, then the position is opened and
        // runs 11 -> 12 -> 13 before breaking down.
        let mut bars: Vec<DailyBar> = (0..21).map(|i| b(i, 11.0, 9.0, 10.0)).collect();
        let entry = bars.len();
        for (n, close) in [11.0, 12.0, 13.0].iter().enumerate() {
            bars.push(c(entry + n, *close));
        }
        bars.push(b(entry + 3, 5.0, 3.0, 4.0));
        let held = OpenLot {
            symbol: "TEST".into(),
            entry_date: day_str(entry),
            entry_price: 11.0,
            shares: 50,
        };
        // The anchor must be the entry bar, not bar zero.
        assert_eq!(
            entry_anchor(&bars, &held.entry_date),
            Some(EntryAnchor {
                index: entry,
                truncated: false
            })
        );
        let s = assess_exits(&bars, Some(&held))
            .into_iter()
            .find(|s| s.trigger == TRIGGER_CHANDELIER)
            .expect("a close through the chandelier must fire");
        assert!(!s.urgent, "the chandelier is the advisory tier");
        // High-water close 13.00, ATR20 2.40 after the wide bar: 13 - 7.2 = 5.8.
        assert!(
            s.rule_fired.contains("highest close since entry 13.00"),
            "{}",
            s.rule_fired
        );
        assert!(
            (s.invalidation_level.unwrap() - 5.8).abs() < 1e-9,
            "{:?}",
            s.invalidation_level
        );
        assert!(!s.rule_fired.contains("fetched window"), "{}", s.rule_fired);
        // Without a lot there is no entry, and so no chandelier at all — the
        // stop is undefined, not defaulted to the start of the series.
        assert!(!assess_exits(&bars, None)
            .iter()
            .any(|s| s.trigger == TRIGGER_CHANDELIER));
    }

    #[test]
    fn a_lot_older_than_the_window_says_its_high_water_mark_is_truncated() {
        // 21 flat bars set ATR20 = 2.0; the drop then has to clear a stop hung
        // 3 ATRs below the 10.00 high-water close, so it is a real collapse.
        let mut bars: Vec<DailyBar> = (0..21).map(|i| b(i, 11.0, 9.0, 10.0)).collect();
        bars.push(b(21, 2.0, 1.0, 1.5));
        let held = OpenLot {
            symbol: "TEST".into(),
            entry_date: "2020-03-02".into(),
            entry_price: 9.0,
            shares: 10,
        };
        let anchor = entry_anchor(&bars, &held.entry_date).unwrap();
        assert_eq!(anchor.index, 0);
        assert!(anchor.truncated, "the entry predates every bar we fetched");
        let s = assess_exits(&bars, Some(&held))
            .into_iter()
            .find(|s| s.trigger == TRIGGER_CHANDELIER)
            .expect("it still fires");
        assert!(
            s.rule_fired.contains("fetched window"),
            "the shortfall is stated, not hidden: {}",
            s.rule_fired
        );
    }

    #[test]
    fn the_overbought_composite_leaves_by_the_same_road() {
        let closes = climb(40);
        let reading = assess(&closes, Some(90.0), 74.0);
        assert!(reading.signal);
        let s = overbought_exit(&reading, &closes).expect("a signal becomes an exit signal");
        assert_eq!(s.trigger, TRIGGER_OVERBOUGHT);
        assert!(!s.urgent);
        assert!(s.invalidation_level.is_some());
        assert!(
            s.invalidation_note.contains("20-day average"),
            "{}",
            s.invalidation_note
        );
        // And no signal means no exit — this path invents nothing. (A dead-flat
        // series is NOT the quiet case: RSI-14 with no down days reads 100 and
        // is genuinely "overbought" by that definition, so the fixture has to
        // be a real chop.)
        let chop: Vec<f64> = (0..40)
            .map(|i| 100.0 + if i % 2 == 0 { 1.5 } else { -1.2 })
            .collect();
        let quiet = assess(&chop, Some(200.0), 74.0);
        assert!(!quiet.signal);
        assert!(overbought_exit(&quiet, &chop).is_none());
    }

    // ── the one path out ──────────────────────────────────────────────────

    fn advisory_signal() -> ExitSignal {
        ExitSignal {
            trigger: TRIGGER_DONCHIAN20_SMA100,
            urgent: false,
            rule_fired: "close 99.00 < dch_lo(20) 100.00 AND close 99.00 < sma(100) 103.99, 2nd \
                         consecutive close"
                .into(),
            invalidation_level: Some(111.0),
            invalidation_note: "a close back above the 20-day high of 111.00 voids this signal"
                .into(),
        }
    }

    #[tokio::test]
    async fn a_second_notice_for_the_same_ticker_and_rule_is_not_filed() {
        let pool = test_pool().await;
        let bars = channel_and_trend_break();
        let held = lot(120.0, 3);
        let reading = assess(&[100.0, 101.0], None, 74.0);
        let signal = advisory_signal();

        let first = file_one_notice(&pool, &held, &signal, &bars, &reading, "2026-09-01")
            .await
            .unwrap();
        assert!(first.is_some(), "the first notice files");

        // The 6-hour sweep, arriving after the close scan.
        let second = file_one_notice(&pool, &held, &signal, &bars, &reading, "2026-09-01")
            .await
            .unwrap();
        assert!(second.is_none(), "the second defers instead of duplicating");
        assert_eq!(open_notice_count(&pool).await, 1);

        // A DIFFERENT rule on the same holding is a different card, not a
        // duplicate — the dedupe axis is the pair.
        let urgent = ExitSignal {
            trigger: TRIGGER_DONCHIAN55,
            urgent: true,
            rule_fired: "close 90.00 < dch_lo(55) 99.00".into(),
            invalidation_level: Some(99.0),
            invalidation_note: "a close back above 99.00 voids this signal".into(),
        };
        assert!(
            file_one_notice(&pool, &held, &urgent, &bars, &reading, "2026-09-01")
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(open_notice_count(&pool).await, 2);

        // And the same rule on a different holding is also its own card.
        let other = OpenLot {
            symbol: "OTHER".into(),
            ..held.clone()
        };
        assert!(
            file_one_notice(&pool, &other, &signal, &bars, &reading, "2026-09-01")
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(open_notice_count(&pool).await, 3);
    }

    async fn open_notice_count(pool: &sqlx::Pool<sqlx::Sqlite>) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM decisions WHERE kind = 'risk_gate' AND status = 'open' \
             AND json_extract(payload_json, '$.sell_notice.symbol') IS NOT NULL",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_resolved_notice_stops_blocking_the_next_one() {
        let pool = test_pool().await;
        let bars = channel_and_trend_break();
        let held = lot(120.0, 3);
        let reading = assess(&[100.0, 101.0], None, 74.0);
        let signal = advisory_signal();

        file_one_notice(&pool, &held, &signal, &bars, &reading, "2026-09-01")
            .await
            .unwrap()
            .unwrap();
        let open = decisions::find_open_sell_notice(&pool, "TEST", signal.trigger)
            .await
            .unwrap()
            .expect("it is open");
        decisions::supersede_decision(&pool, &open.id, "handled elsewhere")
            .await
            .unwrap();
        assert!(
            decisions::find_open_sell_notice(&pool, "TEST", signal.trigger)
                .await
                .unwrap()
                .is_none()
        );
        // The rule can raise again on a later day, which is the point of
        // deduping on OPEN rather than on ever-filed.
        assert!(
            file_one_notice(&pool, &held, &signal, &bars, &reading, "2026-09-08")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn the_notice_replaces_the_toast_rather_than_joining_it() {
        let pool = test_pool().await;
        let mut events = crate::events::subscribe();
        let bars = channel_and_trend_break();
        let held = lot(120.0, 3);
        let reading = assess(&climb(40), Some(90.0), 74.0);
        assert!(reading.signal, "the overbought composite is live here");

        file_one_notice(
            &pool,
            &held,
            &advisory_signal(),
            &bars,
            &reading,
            "2026-09-01",
        )
        .await
        .unwrap()
        .unwrap();
        let overbought = overbought_exit(&reading, &climb(40)).unwrap();
        file_one_notice(&pool, &held, &overbought, &bars, &reading, "2026-09-01")
            .await
            .unwrap()
            .unwrap();

        // The event bus is process-global and other tests share it, so this
        // counts only the thing that must be ABSENT — no other code path in the
        // crate emits a `sell_signal` nudge any more, so a single sighting is a
        // regression. The positive side is counted in the database, which is
        // this test's own.
        let mut nudges = 0;
        while let Ok(event) = events.try_recv() {
            if serde_json::to_value(&event)
                .unwrap_or_default()
                .to_string()
                .contains("sell_signal")
            {
                nudges += 1;
            }
        }
        assert_eq!(
            nudges, 0,
            "the exit signal must not ALSO fire the old proactive nudge"
        );
        assert_eq!(
            open_notice_count(&pool).await,
            2,
            "both signals arrived as decision-inbox cards instead"
        );
    }

    #[tokio::test]
    async fn the_overbought_rule_keeps_its_once_a_day_promise() {
        let pool = test_pool().await;
        let bars = channel_and_trend_break();
        let held = lot(120.0, 3);
        let closes = climb(40);
        let reading = assess(&closes, Some(90.0), 74.0);
        let signal = overbought_exit(&reading, &closes).unwrap();

        assert!(
            file_one_notice(&pool, &held, &signal, &bars, &reading, "2026-09-01")
                .await
                .unwrap()
                .is_some()
        );
        // Answer it, so the open-notice check would let a second one through…
        let open = decisions::find_open_sell_notice(&pool, "TEST", TRIGGER_OVERBOUGHT)
            .await
            .unwrap()
            .unwrap();
        decisions::supersede_decision(&pool, &open.id, "read it")
            .await
            .unwrap();
        // …and the daily record still holds it back until tomorrow.
        assert!(
            file_one_notice(&pool, &held, &signal, &bars, &reading, "2026-09-01")
                .await
                .unwrap()
                .is_none(),
            "the noisiest rule does not re-pitch the same day"
        );
        assert!(
            file_one_notice(&pool, &held, &signal, &bars, &reading, "2026-09-02")
                .await
                .unwrap()
                .is_some(),
            "tomorrow is a new day"
        );
    }

    #[tokio::test]
    async fn advisory_is_tier_one_and_urgent_is_tier_two() {
        let pool = test_pool().await;
        let bars = channel_and_trend_break();
        let held = lot(120.0, 3);
        let reading = assess(&[100.0, 101.0], None, 74.0);

        file_one_notice(
            &pool,
            &held,
            &advisory_signal(),
            &bars,
            &reading,
            "2026-09-01",
        )
        .await
        .unwrap()
        .unwrap();
        let advisory = decisions::find_open_sell_notice(&pool, "TEST", TRIGGER_DONCHIAN20_SMA100)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            advisory.kind, "risk_gate",
            "no new decisions.kind was minted"
        );
        assert_eq!(advisory.tier, 1, "advisory notices are Tier 1");

        let urgent = ExitSignal {
            trigger: TRIGGER_DONCHIAN55,
            urgent: true,
            rule_fired: "close 90.00 < dch_lo(55) 99.00".into(),
            invalidation_level: Some(99.0),
            invalidation_note: "a close back above 99.00 voids this signal".into(),
        };
        file_one_notice(&pool, &held, &urgent, &bars, &reading, "2026-09-01")
            .await
            .unwrap()
            .unwrap();
        let filed = decisions::find_open_sell_notice(&pool, "TEST", TRIGGER_DONCHIAN55)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(filed.tier, 2, "a 55-day breakdown escalates to user-only");
    }

    /// The load-bearing safety claim of this whole module: approving an exit
    /// notice does not sell anything.
    ///
    /// Both `risk_gate` effect arms are guarded on `action_class`, and neither
    /// guard matches the Financier's classes, so an approve falls through to a
    /// no-op. That is a fact about code far from this file, which is exactly
    /// why it is asserted here — if someone later adds a third arm and is not
    /// careful about its guard, this is the test that notices.
    #[tokio::test]
    async fn approving_an_exit_notice_applies_no_effect_at_all() {
        let pool = test_pool().await;
        let bars = channel_and_trend_break();
        let held = lot(120.0, 3);
        let reading = assess(&[100.0, 101.0], None, 74.0);
        file_one_notice(
            &pool,
            &held,
            &advisory_signal(),
            &bars,
            &reading,
            "2026-09-01",
        )
        .await
        .unwrap()
        .unwrap();
        let filed = decisions::find_open_sell_notice(&pool, "TEST", TRIGGER_DONCHIAN20_SMA100)
            .await
            .unwrap()
            .unwrap();

        let (answered, proof) = decisions::answer_decision(
            &pool,
            &filed.id,
            &decisions::DecisionAnswer {
                answer: "approve".into(),
                note: Some("agreed, I'll sell it myself".into()),
                choice_id: None,
                input_text: None,
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .expect("a Tier 1 notice is answerable by the user");
        assert_eq!(answered.answer.as_deref(), Some("approve"));

        let (effect, _) =
            crate::decisions_effects::apply_decision_effect(&pool, &answered, proof, "risk_gate")
                .await
                .expect("the effect path must not error on a class it does not handle");
        assert_eq!(
            effect, None,
            "an approved exit notice must apply NO effect — it is advice, and there is no \
             execution path anywhere in Permagent for it to reach"
        );
    }

    #[tokio::test]
    async fn an_old_risk_gate_payload_without_a_sell_notice_still_parses() {
        // The sub-typed field must be additive: every risk_gate ever written
        // has to keep validating, or the Steward's git-health cards break the
        // day the Financier ships.
        let old = serde_json::json!({
            "action_class": "repo_branch_delete",
            "description": "delete merged branch",
            "requested_by": "steward"
        });
        let parsed: decisions::RiskGatePayload = serde_json::from_value(old).unwrap();
        assert!(parsed.sell_notice.is_none());
        assert!(parsed.repo_target.is_none());
    }

    /// The card the user actually receives, on a fixture whose every number is
    /// checkable by eye. Printed under `--nocapture` so the exact bytes can be
    /// reviewed rather than described.
    #[test]
    fn a_whole_sell_notice_payload() {
        let bars = channel_and_trend_break();
        let held = lot(120.0, 3);
        let signal = assess_exits(&bars, Some(&held))
            .into_iter()
            .find(|s| s.trigger == TRIGGER_DONCHIAN20_SMA100)
            .unwrap();
        let payload = sell_notice_payload(&held, &signal, &bars, "2026-09-01");
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());

        assert_eq!(payload.symbol, "TEST");
        assert_eq!(payload.urgency, "advisory");
        assert_eq!(payload.trigger_close, 99.0);
        assert_eq!(payload.entry_price, 120.0);
        // 99 against a 120 entry on 100 shares.
        assert!((payload.unrealized_pnl_pct - (-17.5)).abs() < 1e-9);
        assert!((payload.unrealized_pnl_usd - (-2_100.0)).abs() < 1e-9);
        assert_eq!(payload.evidence.dch_lo20, Some(100.0));
        assert_eq!(payload.evidence.dch_lo55, Some(99.0));
        // The counter-case is never optional.
        assert_eq!(payload.counter_evidence.len(), 4);
        assert!(payload
            .counter_evidence
            .iter()
            .any(|c| c.contains("earnings proximity is UNKNOWN")));
        assert!(payload.caveat.contains("never an order"));
        // Volume is context, and it is not part of any trigger.
        assert!(!payload.rule_fired.contains("rvol"));
    }

    #[test]
    fn every_headline_fits_the_inbox() {
        for trigger in [
            TRIGGER_DONCHIAN20_SMA100,
            TRIGGER_CHANDELIER,
            TRIGGER_DONCHIAN55,
            TRIGGER_DRAWDOWN,
            TRIGGER_OVERBOUGHT,
        ] {
            let h = headline_for(trigger, "LONGTICKER");
            assert!(h.chars().count() <= decisions::MAX_HEADLINE_CHARS, "{h}");
            assert!(!h.is_empty());
        }
    }

    #[test]
    fn the_trigger_keys_are_stable() {
        // These are the dedupe axis. Renaming one silently re-files every open
        // notice of that kind, so a change has to break this test first.
        assert_eq!(TRIGGER_DONCHIAN20_SMA100, "donchian20_below_sma100");
        assert_eq!(TRIGGER_CHANDELIER, "chandelier_3atr");
        assert_eq!(TRIGGER_DONCHIAN55, "donchian55_breakdown");
        assert_eq!(TRIGGER_DRAWDOWN, "drawdown_from_entry");
        assert_eq!(TRIGGER_OVERBOUGHT, "overbought_composite");
        assert_eq!(DRAWDOWN_URGENT, 0.25);
    }
}
