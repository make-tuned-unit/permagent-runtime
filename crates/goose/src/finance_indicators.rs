//! Deterministic price/volume indicators for the Financier.
//!
//! Pure functions over a slice of adjusted daily bars, oldest → newest, with
//! no I/O — the same shape `pick_loop` and `overbought` already use, so every
//! number here is reproducible from a fixture without a network.
//!
//! ## The parameters are locked. Do not tune them.
//!
//! Every window and threshold below (20/55/252, 50/100/200, ATR20 with k=3.0,
//! rvol 1.5 and 1.25, turnover_ratio > 2.0) was fixed by the F0 research node
//! *before* any of it was run against the user's own pool, and is recorded in
//! `scratchpad/financier-dag/f0-research.md` §5.5. The reason is
//! Bajgrowicz & Scaillet (2012, JFE): for this exact rule family the winning
//! parameters are not identifiable ex ante, and **the search itself
//! manufactures the apparent edge**. Fitting these numbers to the pool would
//! not improve the engine, it would only make its backtest lie. If a
//! parameter must change, change it in F0's spec with a reason, not here.
//!
//! The companion warning, from the same node, belongs on every result this
//! module feeds: *an unexpectedly good backtest is a bug symptom — lookahead,
//! or a channel computed inclusive of the current bar — not a discovery.*
//!
//! ## Current-bar exclusion
//!
//! Channel comparisons use the **prior** N bars, `[t-N, t-1]`, never `[t-N+1,
//! t]`. `close > max(high over a window that contains today)` is trivially
//! near-true on any up day, and a channel that includes its own bar is the
//! single most common way a backtest of these rules invents an edge. The
//! moving averages, ATR, and the liquidity statistics are properties of the
//! series as of the close and *do* include the current bar; each function
//! below states which it is.
//!
//! ## What these are for
//!
//! Conditioning variables and risk controls — not alpha, and not a score to
//! be summed. F0 §7 is blunt that this family adds approximately zero expected
//! return after costs; what it buys is a tradability filter, a stateable
//! drawdown cap, and an auditable evidence pack. Nothing here emits a
//! probability, and nothing here decides anything.

use crate::market_data::DailyBar;

/// Donchian channel windows (F0 §1.5). The Turtle 20/55 pair, unoptimised.
pub const DONCHIAN_SHORT: usize = 20;
pub const DONCHIAN_LONG: usize = 55;
/// The 52-week window behind `chan_pos_252` (George & Hwang, F0 §1.4a).
pub const CHANNEL_LOOKBACK: usize = 252;
/// Simple moving averages (F0 §2.5). "Not 47, not 193."
pub const SMA_FAST: usize = 50;
pub const SMA_MID: usize = 100;
pub const SMA_SLOW: usize = 200;
/// Wilder ATR period and the chandelier multiple (F0 §4.2).
pub const ATR_PERIOD: usize = 20;
pub const CHANDELIER_K: f64 = 3.0;
/// Baseline window for relative volume (F0 §5.0: median, not mean — microcap
/// volume is heavily right-skewed).
pub const RVOL_BASELINE: usize = 50;
/// Volume-confirmation thresholds (F0 §3.4). Single day, then the 3-day form.
pub const RVOL_CONFIRM: f64 = 1.5;
pub const RVOL3_CONFIRM: f64 = 1.25;
/// Liquidity and turnover windows (F0 §3.4, §5.0).
pub const DOLLAR_VOLUME_WINDOW: usize = 20;
pub const TURNOVER_SHORT: usize = 20;
pub const TURNOVER_LONG: usize = 250;
/// Above this, elevated turnover is a *reversal-risk* flag, not a positive:
/// Lee & Swaminathan (2000) and Avramov et al. (2006), F0 §3.3.
pub const REVERSAL_RISK_TURNOVER: f64 = 2.0;
/// Hard history floor. Below this the engine reports, it does not guess.
pub const MIN_BARS: usize = CHANNEL_LOOKBACK;

/// Why an indicator could not be computed.
///
/// There is deliberately no "degraded" variant. F0 §6.5: below the required
/// history the engine emits `insufficient_history` and leaves the value null —
/// silently substituting a 120-day channel would change what the number means
/// while keeping its name, which is the failure mode most likely to fool an
/// LLM judge downstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndicatorError {
    /// Not enough bars. `needed` already accounts for the excluded current bar.
    InsufficientHistory {
        indicator: &'static str,
        needed: usize,
        have: usize,
    },
    /// The series carries no true high/low — every bar's high and low equal
    /// its close, which is what a close-only feed looks like once it has been
    /// poured into OHLC-shaped structs. Donchian and ATR are *undefined* on
    /// such a series; computing them anyway would silently change what they
    /// measure (F0 §6.3).
    MissingHighLow,
    /// The 252-day range collapsed to zero width, so channel position would
    /// divide by zero. A halted or single-priced name, not a signal.
    DegenerateRange,
    /// Every bar in the window had zero volume — halted or untraded. Averaging
    /// those zeros in would understate normal volume and inflate rvol (F0 §6.4).
    NoVolume { window: usize },
    /// A window that should have been non-empty was.
    EmptyWindow,
}

impl std::fmt::Display for IndicatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InsufficientHistory {
                indicator,
                needed,
                have,
            } => write!(
                f,
                "insufficient_history: {indicator} needs {needed} daily bars, {have} available — \
                 the window is not shortened to fit"
            ),
            Self::MissingHighLow => write!(
                f,
                "the series has no true highs or lows; Donchian and ATR are undefined on a \
                 close-only series and closes must not be substituted for them"
            ),
            Self::DegenerateRange => write!(f, "the 252-day high and low are equal"),
            Self::NoVolume { window } => {
                write!(f, "every bar in the {window}-bar volume window traded zero")
            }
            Self::EmptyWindow => write!(f, "the requested window was empty"),
        }
    }
}

impl std::error::Error for IndicatorError {}

type R<T> = Result<T, IndicatorError>;

fn need(indicator: &'static str, needed: usize, have: usize) -> R<()> {
    if have < needed {
        return Err(IndicatorError::InsufficientHistory {
            indicator,
            needed,
            have,
        });
    }
    Ok(())
}

/// Reject a close-only series poured into OHLC structs.
///
/// One flat bar is ordinary (an untraded microcap day); a series in which
/// *every* bar's high and low equal its close is a close series wearing a
/// costume, and every channel and range computed from it would be a different
/// quantity than its name claims.
fn require_true_high_low(bars: &[DailyBar]) -> R<()> {
    if bars.is_empty() {
        return Err(IndicatorError::EmptyWindow);
    }
    if bars.iter().all(|b| b.high == b.close && b.low == b.close) {
        return Err(IndicatorError::MissingHighLow);
    }
    Ok(())
}

/// Median of a slice, by value. Even counts average the two middle points.
fn median(values: &[f64]) -> R<f64> {
    if values.is_empty() {
        return Err(IndicatorError::EmptyWindow);
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    Ok(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// The `n` bars ending at the bar *before* the current one: `[t-n, t-1]`.
fn prior_window<'a>(bars: &'a [DailyBar], n: usize, indicator: &'static str) -> R<&'a [DailyBar]> {
    need(indicator, n + 1, bars.len())?;
    let end = bars.len() - 1;
    Ok(&bars[end - n..end])
}

/// The `n` bars ending at the current one, inclusive: `[t-n+1, t]`.
fn trailing_window<'a>(
    bars: &'a [DailyBar],
    n: usize,
    indicator: &'static str,
) -> R<&'a [DailyBar]> {
    need(indicator, n, bars.len())?;
    Ok(&bars[bars.len() - n..])
}

/// Volumes in a window, with zero-volume (halted/untraded) bars removed.
fn traded_volumes(window: &[DailyBar]) -> R<Vec<f64>> {
    let v: Vec<f64> = window
        .iter()
        .filter(|b| b.volume > 0)
        .map(|b| b.volume as f64)
        .collect();
    if v.is_empty() {
        return Err(IndicatorError::NoVolume {
            window: window.len(),
        });
    }
    Ok(v)
}

// ---------------------------------------------------------------------------
// Moving averages and trend
// ---------------------------------------------------------------------------

/// Simple moving average of the last `n` adjusted closes, **including** the
/// current bar. Use `n` ∈ {50, 100, 200}; nothing here forbids another value,
/// but the spec does (F0 §2.5).
pub fn sma(bars: &[DailyBar], n: usize) -> R<f64> {
    let w = trailing_window(bars, n, "sma")?;
    Ok(w.iter().map(|b| b.close).sum::<f64>() / n as f64)
}

/// `close / sma(200) - 1` — how far above or below its long trend the name is
/// trading, as a signed fraction. Continuous on purpose: F0 §2.4 rejects the
/// binary 50/200 cross for ranking because it discards magnitude, creates mass
/// ties, and flips on noise.
pub fn trend_distance(bars: &[DailyBar]) -> R<f64> {
    let s = sma(bars, SMA_SLOW)?;
    if s == 0.0 {
        return Err(IndicatorError::DegenerateRange);
    }
    Ok(bars[bars.len() - 1].close / s - 1.0)
}

/// [`trend_distance`] clipped to ±0.5 (F0 §5.0 `trend_strength`), so one
/// runaway name cannot dominate a cross-sectional z-score.
pub fn trend_strength(bars: &[DailyBar]) -> R<f64> {
    Ok(trend_distance(bars)?.clamp(-0.5, 0.5))
}

/// `close > sma(200) AND sma(50) > sma(200)` — the tier boundary, not a veto.
/// F0 §2.5: a cheap name below its SMA200 is a legitimate value thesis, it is
/// just not a *trend* thesis, and the notice must say which.
pub fn trend_ok(bars: &[DailyBar]) -> R<bool> {
    let slow = sma(bars, SMA_SLOW)?;
    let fast = sma(bars, SMA_FAST)?;
    Ok(bars[bars.len() - 1].close > slow && fast > slow)
}

// ---------------------------------------------------------------------------
// Donchian channels — prior window only
// ---------------------------------------------------------------------------

/// Highest high over the **prior** `n` bars, `[t-n, t-1]`. Excludes today.
pub fn donchian_hi(bars: &[DailyBar], n: usize) -> R<f64> {
    require_true_high_low(bars)?;
    let w = prior_window(bars, n, "donchian_hi")?;
    Ok(w.iter().fold(f64::MIN, |m, b| m.max(b.high)))
}

/// Lowest low over the **prior** `n` bars, `[t-n, t-1]`. Excludes today.
pub fn donchian_lo(bars: &[DailyBar], n: usize) -> R<f64> {
    require_true_high_low(bars)?;
    let w = prior_window(bars, n, "donchian_lo")?;
    Ok(w.iter().fold(f64::MAX, |m, b| m.min(b.low)))
}

/// Position of today's close inside the prior 252-day true range:
/// `(close - lo252) / (hi252 - lo252)`.
///
/// This is the George & Hwang (2004) nearness-to-52-week-high variable, the
/// one component of this engine with genuine cross-sectional return evidence
/// (F0 §1.4a) — and the reason it is a *continuous* channel position rather
/// than a breakout flag (F0 §1.3 rejects breakout-as-entry outright).
///
/// The window excludes the current bar, so on a genuine new high the result
/// exceeds 1.0 and on a new low it goes below 0.0. That overshoot is
/// information, not an error, and is deliberately not clamped here: clamping
/// would erase exactly the days the ranking cares about. Requires 253 bars.
pub fn chan_pos_252(bars: &[DailyBar]) -> R<f64> {
    require_true_high_low(bars)?;
    need("chan_pos_252", CHANNEL_LOOKBACK + 1, bars.len())?;
    let hi = donchian_hi(bars, CHANNEL_LOOKBACK)?;
    let lo = donchian_lo(bars, CHANNEL_LOOKBACK)?;
    let range = hi - lo;
    if range <= 0.0 {
        return Err(IndicatorError::DegenerateRange);
    }
    Ok((bars[bars.len() - 1].close - lo) / range)
}

// ---------------------------------------------------------------------------
// True range / ATR / chandelier
// ---------------------------------------------------------------------------

/// True range of `bar` given the previous bar's close:
/// `max(high - low, |high - prev_close|, |low - prev_close|)`.
fn true_range(bar: &DailyBar, prev_close: f64) -> f64 {
    (bar.high - bar.low)
        .max((bar.high - prev_close).abs())
        .max((bar.low - prev_close).abs())
}

/// Wilder's Average True Range over `n` bars, ending at the current bar.
///
/// Wilder's original smoothing, not an EMA and not a simple mean: seed with
/// the arithmetic mean of the first `n` true ranges, then
/// `atr = (atr * (n - 1) + tr) / n` for every later bar. A true range needs
/// the previous close, so `n + 1` bars are required for `n` ranges.
pub fn atr(bars: &[DailyBar], n: usize) -> R<f64> {
    require_true_high_low(bars)?;
    need("atr", n + 1, bars.len())?;
    if n == 0 {
        return Err(IndicatorError::EmptyWindow);
    }
    let trs: Vec<f64> = bars
        .windows(2)
        .map(|w| true_range(&w[1], w[0].close))
        .collect();
    let mut value = trs[..n].iter().sum::<f64>() / n as f64;
    for tr in &trs[n..] {
        value = (value * (n as f64 - 1.0) + tr) / n as f64;
    }
    Ok(value)
}

/// `atr(20) / close` — the stop width the name would need, as a fraction of
/// price. F0 §5.1 vetoes above 0.15, where a 3×ATR stop would sit >45% wide.
pub fn atr_pct(bars: &[DailyBar]) -> R<f64> {
    let close = bars[bars.len() - 1].close;
    if close <= 0.0 {
        return Err(IndicatorError::DegenerateRange);
    }
    Ok(atr(bars, ATR_PERIOD)? / close)
}

/// Chandelier trailing stop: `highest_close_since_entry - 3.0 * atr(20)`.
///
/// `entry_index` indexes `bars` at the entry bar; the high-water mark runs
/// from there to the current bar inclusive. Recomputed daily, this ratchets up
/// with the high-water mark and never down.
///
/// Included despite not being in the original brief because it is the only
/// exit here whose trigger *distance means the same thing* in one name as in
/// another (F0 §4.2) — Donchian-20 and SMA100 both trigger at a distance that
/// is an uncontrolled function of the name's own volatility, so a four-position
/// book exited on them carries four different, unknown risk budgets.
///
/// Note this tracks the highest **close**, per F0 §5.3, where the classic
/// chandelier tracks the highest high. That is the spec's choice, not a bug,
/// and it makes the stop marginally tighter.
pub fn chandelier_stop(bars: &[DailyBar], entry_index: usize) -> R<f64> {
    if entry_index >= bars.len() {
        return Err(IndicatorError::EmptyWindow);
    }
    let peak = bars[entry_index..]
        .iter()
        .fold(f64::MIN, |m, b| m.max(b.close));
    Ok(peak - CHANDELIER_K * atr(bars, ATR_PERIOD)?)
}

// ---------------------------------------------------------------------------
// Volume
// ---------------------------------------------------------------------------

/// Today's volume over the median of the **prior** 50 bars' traded volume.
///
/// Median, not mean, because microcap volume is heavily right-skewed and one
/// promoted day would otherwise reset "normal" for the next ten weeks. The
/// baseline excludes the current bar so the ratio is genuinely "against what
/// came before", and zero-volume (halted) bars are excluded from the median
/// rather than counted as zero.
///
/// This is a *confirmation modifier on a breakout bar only* (F0 §3.4).
/// Ranking on volume is rejected: Lee & Swaminathan (2000) find high past
/// turnover predicts **lower** future returns.
pub fn rvol(bars: &[DailyBar]) -> R<f64> {
    let base = median(&traded_volumes(prior_window(bars, RVOL_BASELINE, "rvol")?)?)?;
    if base <= 0.0 {
        return Err(IndicatorError::NoVolume {
            window: RVOL_BASELINE,
        });
    }
    Ok(bars[bars.len() - 1].volume as f64 / base)
}

/// The 3-day form: `sum(volume, 3) / (3 * median(prior 50 traded volumes))`.
///
/// Shares [`rvol`]'s baseline — the 50 bars ending at `t-1` — so the two
/// numbers are directly comparable. Two of the three summed bars therefore
/// also sit inside the baseline; at 2/50 weight that is immaterial, and it is
/// recorded here rather than "fixed" by inventing a second window the spec
/// does not contain.
pub fn rvol3(bars: &[DailyBar]) -> R<f64> {
    need("rvol3", RVOL_BASELINE + 1, bars.len())?;
    let base = median(&traded_volumes(prior_window(
        bars,
        RVOL_BASELINE,
        "rvol3",
    )?)?)?;
    if base <= 0.0 {
        return Err(IndicatorError::NoVolume {
            window: RVOL_BASELINE,
        });
    }
    let recent: f64 = bars[bars.len() - 3..].iter().map(|b| b.volume as f64).sum();
    Ok(recent / (3.0 * base))
}

/// Today's volume over the **mean** of the prior 20 bars' traded volume.
///
/// The plainer "is today busy?" reading, kept alongside [`rvol`] because it is
/// what the G1 brief asked for in words, while [`rvol`] is what F0 §5.0 locked
/// in symbols. They are different quantities — a mean over 20 skewed days sits
/// well above the median over 50 — so they are named differently and the
/// 1.5/1.25 thresholds belong to [`rvol`]/[`rvol3`], not to this one.
pub fn rvol_20d_mean(bars: &[DailyBar]) -> R<f64> {
    let traded = traded_volumes(prior_window(bars, DOLLAR_VOLUME_WINDOW, "rvol_20d_mean")?)?;
    let base = traded.iter().sum::<f64>() / traded.len() as f64;
    if base <= 0.0 {
        return Err(IndicatorError::NoVolume {
            window: DOLLAR_VOLUME_WINDOW,
        });
    }
    Ok(bars[bars.len() - 1].volume as f64 / base)
}

/// `median(close * volume, 20)` over the last 20 bars, current bar included.
///
/// A **tradability** measure, never an alpha term: Amihud (2002) and Novy-Marx
/// & Velikov (2016) support it as a hard eligibility gate that removes names
/// where the round trip costs more than any plausible edge. Anything that
/// surfaces this number downstream must label it that way, or an LLM judge
/// will read it as "more volume is better", which F0 §3.3 says is false.
///
/// Includes the current bar because it describes liquidity as of the close,
/// which is knowable at scan time and is not compared against itself.
pub fn dollar_volume_20d_median(bars: &[DailyBar]) -> R<f64> {
    let w = trailing_window(bars, DOLLAR_VOLUME_WINDOW, "dollar_volume_20d_median")?;
    let dv: Vec<f64> = w
        .iter()
        .filter(|b| b.volume > 0)
        .map(|b| b.close * b.volume as f64)
        .collect();
    if dv.is_empty() {
        return Err(IndicatorError::NoVolume {
            window: DOLLAR_VOLUME_WINDOW,
        });
    }
    median(&dv)
}

/// `median(volume, 20) / median(volume, 250)` — sustained elevated turnover.
///
/// Deliberately *inverted* relative to intuition. This is not a positive
/// signal; above [`REVERSAL_RISK_TURNOVER`] it is a reversal-risk flag, per
/// Lee & Swaminathan (2000) and Avramov, Chordia & Goyal (2006), who find the
/// largest reversals in exactly the high-turnover, low-liquidity names this
/// pool is made of (F0 §3.3). It must never be collapsed together with
/// [`rvol`] into "volume": event-day abnormal volume and sustained elevated
/// turnover are different quantities pointing in opposite directions.
pub fn turnover_ratio(bars: &[DailyBar]) -> R<f64> {
    let short = median(&traded_volumes(trailing_window(
        bars,
        TURNOVER_SHORT,
        "turnover_ratio",
    )?)?)?;
    let long = median(&traded_volumes(trailing_window(
        bars,
        TURNOVER_LONG,
        "turnover_ratio",
    )?)?)?;
    if long <= 0.0 {
        return Err(IndicatorError::NoVolume {
            window: TURNOVER_LONG,
        });
    }
    Ok(short / long)
}

/// `turnover_ratio > 2.0`. See [`turnover_ratio`] for why this is a warning.
pub fn reversal_risk(bars: &[DailyBar]) -> R<bool> {
    Ok(turnover_ratio(bars)? > REVERSAL_RISK_TURNOVER)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a bar. Volume defaults are supplied per-test.
    fn bar(high: f64, low: f64, close: f64, volume: u64) -> DailyBar {
        DailyBar {
            epoch_seconds: 0,
            open: close,
            high,
            low,
            close,
            volume,
        }
    }

    /// A bar whose high/low straddle the close by ±1, so `require_true_high_low`
    /// is satisfied and the close drives the arithmetic.
    fn c(close: f64, volume: u64) -> DailyBar {
        bar(close + 1.0, close - 1.0, close, volume)
    }

    fn series(closes: &[f64]) -> Vec<DailyBar> {
        closes.iter().map(|&x| c(x, 1_000)).collect()
    }

    // -- moving averages ---------------------------------------------------

    #[test]
    fn sma_is_the_mean_of_the_trailing_closes_inclusive_of_today() {
        let bars = series(&[1., 2., 3., 4., 5., 6., 7., 8., 9., 10.]);
        // last 5 closes: 6 + 7 + 8 + 9 + 10 = 40; 40 / 5 = 8
        assert_eq!(sma(&bars, 5).unwrap(), 8.0);
        // all 10: 1+2+…+10 = 55; 55 / 10 = 5.5
        assert_eq!(sma(&bars, 10).unwrap(), 5.5);
    }

    #[test]
    fn trend_distance_is_close_over_sma200_minus_one() {
        // 200 closes all 100.0, then one more at 100.0 keeps sma200 = 100…
        let mut closes = vec![100.0; 199];
        closes.push(120.0);
        let bars = series(&closes);
        // sma200 = (199 * 100 + 120) / 200 = (19900 + 120) / 200 = 20020/200 = 100.1
        assert!((sma(&bars, 200).unwrap() - 100.1).abs() < 1e-12);
        // 120 / 100.1 - 1 = 0.1988011988…
        let d = trend_distance(&bars).unwrap();
        assert!((d - (120.0 / 100.1 - 1.0)).abs() < 1e-12);
        assert!((d - 0.198801198801).abs() < 1e-9, "got {d}");
    }

    #[test]
    fn trend_strength_clips_at_half() {
        let mut closes = vec![10.0; 199];
        closes.push(1_000.0); // close/sma200 - 1 is ~49, far past the clip
        let bars = series(&closes);
        assert_eq!(trend_strength(&bars).unwrap(), 0.5);
    }

    #[test]
    fn trend_ok_needs_both_price_and_the_fast_average_above_the_slow_one() {
        // Rising ramp: close > sma200 and sma50 > sma200.
        let rising: Vec<f64> = (0..200).map(|i| 10.0 + i as f64).collect();
        assert!(trend_ok(&series(&rising)).unwrap());
        // Falling ramp: neither holds.
        let falling: Vec<f64> = (0..200).map(|i| 210.0 - i as f64).collect();
        assert!(!trend_ok(&series(&falling)).unwrap());
    }

    // -- Donchian, and the current-bar exclusion ---------------------------

    #[test]
    fn donchian_uses_the_prior_window_and_excludes_todays_bar() {
        // index:      0     1     2     3     4     5 (current)
        // high:      10    12    11    15     9    20
        // low:        5     6     4     7     3     1
        let bars = vec![
            bar(10.0, 5.0, 8.0, 100),
            bar(12.0, 6.0, 9.0, 100),
            bar(11.0, 4.0, 7.0, 100),
            bar(15.0, 7.0, 12.0, 100),
            bar(9.0, 3.0, 5.0, 100),
            bar(20.0, 1.0, 18.0, 100),
        ];
        // Prior 4 bars are indices 1..=4: highs 12, 11, 15, 9 -> max 15.
        // Today's high of 20 is the largest in the whole series and MUST NOT win.
        assert_eq!(donchian_hi(&bars, 4).unwrap(), 15.0);
        // Prior 4 lows: 6, 4, 7, 3 -> min 3. Today's low of 1 is excluded.
        assert_eq!(donchian_lo(&bars, 4).unwrap(), 3.0);
        // Proof of the exclusion, stated as the values an inclusive window
        // would have produced: max 20, min 1.
        let inclusive_hi = bars[2..].iter().fold(f64::MIN, |m, b| m.max(b.high));
        let inclusive_lo = bars[2..].iter().fold(f64::MAX, |m, b| m.min(b.low));
        assert_eq!((inclusive_hi, inclusive_lo), (20.0, 1.0));
        assert_ne!(donchian_hi(&bars, 4).unwrap(), inclusive_hi);
        assert_ne!(donchian_lo(&bars, 4).unwrap(), inclusive_lo);
    }

    #[test]
    fn donchian_needs_one_bar_more_than_its_window() {
        let bars = series(&[1., 2., 3., 4.]);
        assert_eq!(
            donchian_hi(&bars, 4).unwrap_err(),
            IndicatorError::InsufficientHistory {
                indicator: "donchian_hi",
                needed: 5,
                have: 4
            }
        );
        assert!(donchian_hi(&bars, 3).is_ok());
    }

    /// The single most important test in this file. F0 §7: "if a backtest on
    /// the user's own pool shows a large return improvement, the correct first
    /// hypothesis is a bug — lookahead through unadjusted prices, survivorship
    /// in the scanner pool, or a channel computed inclusive of the current bar
    /// — or overfit. Not a discovery." This pins the third one.
    #[test]
    fn chan_pos_252_excludes_the_current_bar() {
        // 252 prior bars: bar 0 carries the extremes (high 300, low 100),
        // every other prior bar sits inside them (high 250, low 150).
        let mut bars = vec![bar(300.0, 100.0, 200.0, 1_000)];
        for _ in 1..252 {
            bars.push(bar(250.0, 150.0, 200.0, 1_000));
        }
        // Current bar: a blowout day whose own high/low would swallow the range.
        bars.push(bar(400.0, 50.0, 250.0, 1_000));
        assert_eq!(bars.len(), 253);

        // Prior window: hi252 = 300, lo252 = 100, range = 200.
        // (250 - 100) / 200 = 150 / 200 = 0.75
        assert_eq!(chan_pos_252(&bars).unwrap(), 0.75);

        // Had the current bar been included: hi = 400, lo = 50, range = 350,
        // (250 - 50) / 350 = 200 / 350 = 0.571428…  — a different number, and
        // the one an inclusive implementation would have returned.
        let incl_hi = bars.iter().fold(f64::MIN, |m, b| m.max(b.high));
        let incl_lo = bars.iter().fold(f64::MAX, |m, b| m.min(b.low));
        let inclusive = (250.0 - incl_lo) / (incl_hi - incl_lo);
        assert!((inclusive - 0.5714285714).abs() < 1e-9);
        assert_ne!(chan_pos_252(&bars).unwrap(), inclusive);
    }

    #[test]
    fn chan_pos_252_is_not_clamped_on_a_genuine_new_high() {
        let mut bars = vec![bar(300.0, 100.0, 200.0, 1_000)];
        for _ in 1..252 {
            bars.push(bar(250.0, 150.0, 200.0, 1_000));
        }
        // Close above the prior 252-day high: (350 - 100) / 200 = 1.25
        bars.push(bar(360.0, 340.0, 350.0, 1_000));
        assert_eq!(chan_pos_252(&bars).unwrap(), 1.25);
    }

    #[test]
    fn chan_pos_252_fails_loudly_below_252_prior_bars() {
        // Exactly 252 bars is one short: the window is the 252 bars BEFORE today.
        let bars = series(&(0..252).map(|i| 10.0 + i as f64).collect::<Vec<_>>());
        assert_eq!(
            chan_pos_252(&bars).unwrap_err(),
            IndicatorError::InsufficientHistory {
                indicator: "chan_pos_252",
                needed: 253,
                have: 252
            }
        );
        // And it is never quietly shortened to a window that would fit.
        let short = series(&(0..120).map(|i| 10.0 + i as f64).collect::<Vec<_>>());
        assert!(matches!(
            chan_pos_252(&short),
            Err(IndicatorError::InsufficientHistory { .. })
        ));
        // 253 bars is enough.
        let ok = series(&(0..253).map(|i| 10.0 + i as f64).collect::<Vec<_>>());
        assert!(chan_pos_252(&ok).is_ok());
    }

    #[test]
    fn a_close_only_series_in_ohlc_clothing_is_refused() {
        // What a close-only feed looks like once someone substitutes the close
        // for the high and the low. Donchian and ATR are undefined here.
        let closes: Vec<DailyBar> = (0..300)
            .map(|i| {
                let px = 10.0 + i as f64;
                bar(px, px, px, 1_000)
            })
            .collect();
        assert_eq!(
            donchian_hi(&closes, 20).unwrap_err(),
            IndicatorError::MissingHighLow
        );
        assert_eq!(
            donchian_lo(&closes, 20).unwrap_err(),
            IndicatorError::MissingHighLow
        );
        assert_eq!(
            atr(&closes, 20).unwrap_err(),
            IndicatorError::MissingHighLow
        );
        assert_eq!(
            chan_pos_252(&closes).unwrap_err(),
            IndicatorError::MissingHighLow
        );
        // A single flat bar inside an otherwise real series is fine — that is
        // an untraded microcap day, not a broken feed.
        let mut mixed = closes.clone();
        mixed[7] = bar(20.0, 5.0, 12.0, 1_000);
        assert!(donchian_hi(&mixed, 20).is_ok());
    }

    // -- ATR ---------------------------------------------------------------

    #[test]
    fn atr_follows_wilders_smoothing_by_hand() {
        // bar0: h=10 l=8  c=9
        // bar1: h=12 l=9  c=11  TR1 = max(12-9=3, |12-9|=3, |9-9|=0)   = 3
        // bar2: h=13 l=11 c=12  TR2 = max(13-11=2, |13-11|=2, |11-11|=0) = 2
        // bar3: h=14 l=10 c=10  TR3 = max(14-10=4, |14-12|=2, |10-12|=2) = 4
        // bar4: h=11 l=9  c=10  TR4 = max(11-9=2,  |11-10|=1, |9-10|=1)  = 2
        let bars = vec![
            bar(10.0, 8.0, 9.0, 100),
            bar(12.0, 9.0, 11.0, 100),
            bar(13.0, 11.0, 12.0, 100),
            bar(14.0, 10.0, 10.0, 100),
            bar(11.0, 9.0, 10.0, 100),
        ];
        // Seed = mean(TR1, TR2, TR3) = (3 + 2 + 4) / 3 = 3.0
        // Step  = (3.0 * (3 - 1) + TR4) / 3 = (6 + 2) / 3 = 8/3 = 2.666666…
        let a = atr(&bars, 3).unwrap();
        assert!((a - 8.0 / 3.0).abs() < 1e-12, "got {a}");
        // Seeding alone, with exactly n + 1 bars, is the plain mean of n TRs.
        assert_eq!(atr(&bars[..4], 3).unwrap(), 3.0);
    }

    #[test]
    fn atr20_seeds_on_twenty_ranges_then_smooths() {
        // 21 bars, every bar spanning exactly 2.0 with a flat close, so every
        // one of the 20 true ranges is 2.0 and the seed is 2.0 exactly.
        let mut bars: Vec<DailyBar> = (0..21).map(|_| bar(11.0, 9.0, 10.0, 100)).collect();
        assert_eq!(atr(&bars, ATR_PERIOD).unwrap(), 2.0);
        // Add one wide bar: h=32, l=10, prev close 10 -> TR = max(22, 22, 0) = 22.
        // (2.0 * 19 + 22) / 20 = (38 + 22) / 20 = 60 / 20 = 3.0
        bars.push(bar(32.0, 10.0, 10.0, 100));
        assert_eq!(atr(&bars, ATR_PERIOD).unwrap(), 3.0);
        // atr_pct = 3.0 / close 10.0 = 0.30 — above F0's 0.15 veto, as intended.
        assert!((atr_pct(&bars).unwrap() - 0.30).abs() < 1e-12);
    }

    #[test]
    fn atr_of_a_constant_true_range_is_that_constant() {
        // Wilder's smoothing is a weighted mean, so a constant input is a
        // fixed point of it however long the series runs.
        let bars: Vec<DailyBar> = (0..300).map(|_| bar(15.0, 10.0, 12.0, 100)).collect();
        // Every TR after the first bar: max(5, |15-12|=3, |10-12|=2) = 5.
        assert!((atr(&bars, ATR_PERIOD).unwrap() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn atr_fails_loudly_without_a_previous_close_for_every_range() {
        let bars: Vec<DailyBar> = (0..20).map(|_| bar(11.0, 9.0, 10.0, 100)).collect();
        assert_eq!(
            atr(&bars, ATR_PERIOD).unwrap_err(),
            IndicatorError::InsufficientHistory {
                indicator: "atr",
                needed: 21,
                have: 20
            }
        );
    }

    #[test]
    fn chandelier_hangs_three_atrs_below_the_high_water_close() {
        // 21 flat bars (ATR20 = 2.0), then the position runs up and pulls back.
        let mut bars: Vec<DailyBar> = (0..21).map(|_| bar(11.0, 9.0, 10.0, 100)).collect();
        let entry = bars.len(); // enter on the next bar
                                // One point a day keeps every true range at exactly 2.0, so the ATR
                                // the stop is built on stays 2.0 and the stop's arithmetic is visible:
                                // up day   -> high-low = 2, |high-prev| = |c+1-(c-1)| = 2, |low-prev| = 0
                                // down day -> high-low = 2, |high-prev| = 0,              |low-prev| = 2
        for close in [11.0, 12.0, 13.0, 12.0] {
            bars.push(bar(close + 1.0, close - 1.0, close, 100));
        }
        // Recheck the ATR the stop is built on before trusting the stop.
        let a = atr(&bars, ATR_PERIOD).unwrap();
        assert!((a - 2.0).abs() < 1e-9, "atr drifted: {a}");
        // Highest close since entry = 13.0; 13.0 - 3.0 * 2.0 = 7.0
        let stop = chandelier_stop(&bars, entry).unwrap();
        assert!((stop - 7.0).abs() < 1e-9, "got {stop}");
        // It ratchets: today's close of 13.0 did not lower the 16.0 peak.
        assert!(stop > bars.last().unwrap().close - 3.0 * a);
    }

    // -- volume ------------------------------------------------------------

    #[test]
    fn rvol_is_today_over_the_median_of_the_prior_fifty() {
        // 50 prior bars: 25 at volume 100 and 25 at 300, interleaved so the
        // order cannot matter. Sorted, the two middle points are 100 and 300,
        // so the median is (100 + 300) / 2 = 200.
        let mut bars: Vec<DailyBar> = (0..50)
            .map(|i| c(10.0, if i % 2 == 0 { 100 } else { 300 }))
            .collect();
        bars.push(c(10.0, 300)); // today
        assert_eq!(bars.len(), 51);
        // 300 / 200 = 1.5 — exactly the confirmation threshold.
        assert_eq!(rvol(&bars).unwrap(), 1.5);
        assert!(rvol(&bars).unwrap() >= RVOL_CONFIRM);
    }

    #[test]
    fn rvol_baseline_excludes_today_so_a_spike_cannot_dilute_itself() {
        let mut bars: Vec<DailyBar> = (0..50).map(|_| c(10.0, 100)).collect();
        bars.push(c(10.0, 10_000)); // a 100x day
                                    // Baseline is the prior 50, all 100 -> median 100. 10000 / 100 = 100.
        assert_eq!(rvol(&bars).unwrap(), 100.0);
        // Had today been inside the baseline the median would still be 100
        // here, but the exclusion is what makes that guaranteed rather than
        // lucky: the prior window is the only thing the ratio is measured
        // against, so the reading is stable no matter how large today is.
        let mut bigger = bars.clone();
        *bigger.last_mut().unwrap() = c(10.0, 1_000_000);
        assert_eq!(rvol(&bigger).unwrap(), 10_000.0);
    }

    #[test]
    fn rvol3_sums_three_days_against_the_same_baseline() {
        let mut bars: Vec<DailyBar> = (0..50).map(|_| c(10.0, 100)).collect();
        bars.push(c(10.0, 150)); // index 50
        bars.push(c(10.0, 200)); // index 51, today
        assert_eq!(bars.len(), 52);
        // Baseline = the 50 bars ending at t-1, i.e. indices 1..=50: forty-nine
        // 100s and the 150. Sorted, the two middle points are both 100, so the
        // median is 100.
        // Last three bars are indices 49 (100), 50 (150), 51 (200):
        // (100 + 150 + 200) / (3 * 100) = 450 / 300 = 1.5
        let r = rvol3(&bars).unwrap();
        assert!((r - 1.5).abs() < 1e-12, "got {r}");
        assert!(r >= RVOL3_CONFIRM);
    }

    #[test]
    fn rvol_20d_mean_is_the_briefs_plainer_reading_and_a_different_number() {
        let mut bars: Vec<DailyBar> = (0..20).map(|_| c(10.0, 1_000)).collect();
        bars.push(c(10.0, 2_500));
        // Prior 20 all 1000 -> mean 1000. 2500 / 1000 = 2.5
        assert_eq!(rvol_20d_mean(&bars).unwrap(), 2.5);

        // On a right-skewed series the mean-over-20 and median-over-50 readings
        // diverge, which is why they are named separately and why the 1.5
        // threshold belongs to `rvol`.
        let mut skewed: Vec<DailyBar> = (0..50)
            .map(|i| c(10.0, if i == 49 { 100_000 } else { 100 }))
            .collect();
        skewed.push(c(10.0, 300));
        // median of the prior 50 = 100 -> rvol = 3.0
        assert_eq!(rvol(&skewed).unwrap(), 3.0);
        // mean of the prior 20 = (19 * 100 + 100000) / 20 = 101900 / 20 = 5095
        // 300 / 5095 = 0.05888…
        let m = rvol_20d_mean(&skewed).unwrap();
        assert!((m - 300.0 / 5095.0).abs() < 1e-12, "got {m}");
        assert!(m < 0.06 && rvol(&skewed).unwrap() > 2.9);
    }

    #[test]
    fn zero_volume_halted_days_are_excluded_from_the_baseline_not_averaged_in() {
        // 50 prior bars: 25 halted (volume 0) and 25 that traded 200.
        let mut bars: Vec<DailyBar> = (0..50)
            .map(|i| c(10.0, if i % 2 == 0 { 0 } else { 200 }))
            .collect();
        bars.push(c(10.0, 400));
        // Median over the 25 that traded = 200, so 400 / 200 = 2.0.
        // Counting the zeros the median would have been (0 + 200)/2 = 100 and
        // rvol would have read 4.0 — twice the truth.
        assert_eq!(rvol(&bars).unwrap(), 2.0);
    }

    #[test]
    fn an_entirely_halted_window_fails_rather_than_dividing_by_zero() {
        let mut bars: Vec<DailyBar> = (0..50).map(|_| c(10.0, 0)).collect();
        bars.push(c(10.0, 400));
        assert_eq!(
            rvol(&bars).unwrap_err(),
            IndicatorError::NoVolume { window: 50 }
        );
    }

    #[test]
    fn dollar_volume_is_the_median_of_the_last_twenty_close_times_volume() {
        // 10 bars at 2.00 x 100 = 200, then 10 at 4.00 x 100 = 400.
        let mut bars: Vec<DailyBar> = (0..10).map(|_| c(2.0, 100)).collect();
        bars.extend((0..10).map(|_| c(4.0, 100)));
        // Sorted: ten 200s then ten 400s; middle two are 200 and 400 -> 300.
        assert_eq!(dollar_volume_20d_median(&bars).unwrap(), 300.0);
    }

    #[test]
    fn dollar_volume_needs_twenty_bars() {
        let bars: Vec<DailyBar> = (0..19).map(|_| c(2.0, 100)).collect();
        assert_eq!(
            dollar_volume_20d_median(&bars).unwrap_err(),
            IndicatorError::InsufficientHistory {
                indicator: "dollar_volume_20d_median",
                needed: 20,
                have: 19
            }
        );
    }

    #[test]
    fn turnover_ratio_flags_sustained_elevation_as_a_risk_not_a_positive() {
        // 250 bars: the oldest 230 traded 100, the most recent 20 traded 400.
        let mut bars: Vec<DailyBar> = (0..230).map(|_| c(10.0, 100)).collect();
        bars.extend((0..20).map(|_| c(10.0, 400)));
        assert_eq!(bars.len(), 250);
        // median of the last 20 = 400.
        // median of all 250: sorted, 230 hundreds then 20 four-hundreds; the
        // 125th and 126th points are both 100 -> 100.
        // 400 / 100 = 4.0
        assert_eq!(turnover_ratio(&bars).unwrap(), 4.0);
        assert!(reversal_risk(&bars).unwrap(), "4.0 > 2.0 is a warning");

        // A quiet name is not flagged.
        let calm: Vec<DailyBar> = (0..250).map(|_| c(10.0, 100)).collect();
        assert_eq!(turnover_ratio(&calm).unwrap(), 1.0);
        assert!(!reversal_risk(&calm).unwrap());
    }

    #[test]
    fn turnover_ratio_needs_the_full_250_bar_denominator() {
        let bars: Vec<DailyBar> = (0..249).map(|_| c(10.0, 100)).collect();
        assert_eq!(
            turnover_ratio(&bars).unwrap_err(),
            IndicatorError::InsufficientHistory {
                indicator: "turnover_ratio",
                needed: 250,
                have: 249
            }
        );
    }

    // -- a real payload ----------------------------------------------------

    /// End-to-end on a captured Yahoo `300d` response for LEE — a sub-$10 name
    /// from the actual pool, so the hard cases (a null bar, a 300-bar window,
    /// an adjustment factor) are exercised on real data rather than on a
    /// series built to be convenient.
    ///
    /// The expected values were derived independently, outside Rust, from this
    /// frozen fixture: they are a second implementation of the same
    /// definitions, not a snapshot of this one. A number that changes here
    /// means the engine changed, because the fixture cannot.
    #[test]
    fn the_whole_engine_runs_on_a_captured_yahoo_payload() {
        let body: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/yahoo-chart-LEE-300d.json"))
                .expect("fixture parses");
        let bars = crate::market_data::parse_bars(&body).expect("fixture yields bars");

        // 300 timestamps, one of them a no-trade day with null prices. It is
        // dropped, not zero-filled — and 299 still clears the 253-bar floor.
        assert_eq!(bars.len(), 299);
        assert!(bars.len() > MIN_BARS);
        for b in &bars {
            assert!(b.high >= b.low, "{b:?}");
            assert!(b.high >= b.close && b.low <= b.close, "{b:?}");
        }

        let close = bars.last().unwrap().close;
        let near = |got: f64, want: f64| {
            assert!(
                (got - want).abs() < 1e-9,
                "got {got}, expected {want} (delta {})",
                got - want
            );
        };
        near(close, 8.0);
        near(
            donchian_hi(&bars, DONCHIAN_SHORT).unwrap(),
            9.300_000_190_734_863,
        );
        near(
            donchian_lo(&bars, DONCHIAN_SHORT).unwrap(),
            7.300_000_190_734_863,
        );
        near(
            donchian_hi(&bars, DONCHIAN_LONG).unwrap(),
            11.050_000_190_734_863,
        );
        near(
            donchian_lo(&bars, DONCHIAN_LONG).unwrap(),
            6.849_999_904_632_568,
        );
        near(chan_pos_252(&bars).unwrap(), 0.545_667_444_560_772);
        near(sma(&bars, SMA_FAST).unwrap(), 8.292_799_987_792_968);
        near(sma(&bars, SMA_MID).unwrap(), 8.686_799_998_283_385);
        near(sma(&bars, SMA_SLOW).unwrap(), 7.385_099_995_136_261);
        near(atr(&bars, ATR_PERIOD).unwrap(), 0.491_705_657_397_318_3);
        near(atr_pct(&bars).unwrap(), 0.061_463_207_174_664_79);
        near(rvol(&bars).unwrap(), 0.584_033_613_445_378_2);
        near(
            dollar_volume_20d_median(&bars).unwrap(),
            284_580.513_191_223_14,
        );
        near(turnover_ratio(&bars).unwrap(), 0.832_731_648_616_125_1);

        // The derived reads the Financier will act on, stated plainly.
        assert!(
            trend_ok(&bars).unwrap(),
            "8.00 > sma200 7.385, sma50 > sma200"
        );
        assert!(
            !reversal_risk(&bars).unwrap(),
            "turnover 0.83 is not elevated"
        );
        assert!(
            atr_pct(&bars).unwrap() <= 0.15,
            "F0 §5.1 G4: a 3xATR stop here is ~18% wide, inside the veto"
        );
        assert!(
            rvol(&bars).unwrap() < RVOL_CONFIRM,
            "a quiet day is not confirmation"
        );
    }

    // -- shared helpers ----------------------------------------------------

    #[test]
    fn median_averages_the_two_middle_points_of_an_even_window() {
        assert_eq!(median(&[3.0, 1.0, 2.0]).unwrap(), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]).unwrap(), 2.5);
        assert_eq!(median(&[7.0]).unwrap(), 7.0);
        assert_eq!(median(&[]).unwrap_err(), IndicatorError::EmptyWindow);
    }

    #[test]
    fn the_locked_parameters_are_the_ones_f0_locked() {
        // A tripwire, not a tautology: if someone "improves" a window, this
        // fails and sends them to f0-research.md §5.5 before the change lands.
        assert_eq!(
            (DONCHIAN_SHORT, DONCHIAN_LONG, CHANNEL_LOOKBACK),
            (20, 55, 252)
        );
        assert_eq!((SMA_FAST, SMA_MID, SMA_SLOW), (50, 100, 200));
        assert_eq!((ATR_PERIOD, CHANDELIER_K), (20, 3.0));
        assert_eq!((RVOL_CONFIRM, RVOL3_CONFIRM), (1.5, 1.25));
        assert_eq!(REVERSAL_RISK_TURNOVER, 2.0);
        assert_eq!(MIN_BARS, 252);
    }
}
