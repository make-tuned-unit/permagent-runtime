//! Tomorrow's pick — the Financier's close-of-day judgment.
//!
//! Picker ranks. The loop gate keeps the farm honest. Opus may choose **one**
//! name from the surviving candidates for tomorrow's open, or none. A ticker
//! that was not in the list is refused. Silence is the honest answer when
//! nothing clears.
//!
//! ## The indicators are one-way
//!
//! Every candidate reaching the judge has already passed Picker's ranking AND
//! `pick_loop`'s significance gate. The indicator layer added here may only
//! ever **remove** or **reorder** those survivors — it can veto a name on
//! liquidity, and it can rank one above another, but nothing it computes can
//! put a loop-gate failure back in front of Opus. That asymmetry is the whole
//! safety argument for adding a second scoring surface to a pipeline that
//! already had one, and [`rank_candidates`] enforces it structurally rather
//! than by convention.
//!
//! ## The combination is tiered, not summed
//!
//! F0 §5.2. Each component is evidenced as a *conditional*: channel position
//! is a ranking variable (George & Hwang), trend is a filter (ap Gwilym et
//! al.), volume is a confirmation modifier (Cooper; Blume et al.), and dollar
//! volume is a tradability veto (Amihud; Novy-Marx & Velikov) that is **not an
//! alpha term**. Adding them into one score would assert a substitutability
//! none of that evidence supports, so the tiers are ordinal and only the
//! within-tier tiebreak is arithmetic.
//!
//! ## Missing indicators are stated, not silently dropped
//!
//! A name with fewer than 253 bars, or a feed with no true highs and lows,
//! gets an evidence block that says so in words. It is **not** vetoed and not
//! quietly removed: it cleared the loop gate on its close series, and the
//! Financier can judge it on the scanner's own case as it did before this
//! layer existed. What it must never do is see a plausible-looking number that
//! was computed over a shortened window — F0 §6.5, the failure mode most
//! likely to fool an LLM judge.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};

use crate::agents::reply_parts::AccountedFastCompletion;
use crate::conversation::message::Message;
use crate::cost_router::{role_map, WorkflowRole};
use crate::finance_indicators as ind;
use crate::market_data::{self, DailyBar};
use crate::pick_loop;
use crate::picker;

pub const OPUS_PROVIDER: &str = "anthropic";
pub const OPUS_MODEL: &str = "claude-opus-4-8";
const MAX_CANDIDATES: usize = 8;

/// F0 §5.1 G2: `dv20 >= 25 * intended_position_$`. Novy-Marx & Velikov — the
/// round trip must be small against the name's daily traded dollars.
pub const DOLLAR_VOLUME_MULTIPLE: f64 = 25.0;
/// F0 §5.1 G3. The literature almost universally screens at $5; this pool sits
/// below that by design, so the floor is relaxed to $1 AND every name that
/// passed on the relaxed threshold is recorded as such
/// ([`CandidateEvidence::relaxed_price_gate`]) so F5 can measure the two
/// groups separately instead of blending them.
pub const MIN_PRICE: f64 = 1.00;
/// The threshold the academic work actually uses. Not a gate here — a label.
pub const LITERATURE_PRICE_FLOOR: f64 = 5.00;
/// F0 §5.1 G4: above this a 3×ATR stop would sit more than 45% wide.
pub const MAX_ATR_PCT: f64 = 0.15;
/// F0 §5.2: a breakout counts as "fresh" for this many sessions.
pub const BREAKOUT_FRESH_SESSIONS: usize = 5;
/// F0 §5.2 tier boundaries on `chan_pos_252`.
pub const CHAN_POS_TIER_A: f64 = 0.80;
pub const CHAN_POS_TIER_B: f64 = 0.60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DailyPick {
    pub day: String,
    pub as_of: String,
    pub ticker: Option<String>,
    pub company_name: Option<String>,
    pub why: String,
    pub model: Option<String>,
    pub candidate_count: i64,
}

/// The exact words an unavailable evidence block leads with. A single
/// constant so the judge's prompt, the tests, and the block itself cannot
/// drift apart into three different phrasings of the same fact.
pub const INDICATORS_UNAVAILABLE: &str = "insufficient history — indicators unavailable";

/// One candidate's indicator evidence, as the Financier sees it.
///
/// Every number is `Option`: absent means "not computable from the history
/// that came back", never zero and never a shortened window. `status` says in
/// words which of those two worlds this block is in, and is the first thing in
/// the serialized form for exactly that reason.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CandidateEvidence {
    /// Plain-language state of this block. Either the tier line, or
    /// [`INDICATORS_UNAVAILABLE`] with the reason.
    pub status: String,
    /// F0 §5.2 tier — `"A"`..`"D"`. `None` when the indicators are unavailable
    /// or a veto fired; an untiered name is not a Tier D name.
    pub tier: Option<String>,
    /// False when a hard eligibility veto fired. A name with `eligible: false`
    /// is refused by [`parse_judgment`] even if the model names it.
    pub eligible: bool,
    /// Why, in words, with both numbers substituted. Empty when eligible.
    pub vetoes: Vec<String>,
    /// George & Hwang nearness-to-52-week-high, the one component with genuine
    /// cross-sectional return evidence. Ranks WITHIN a tier.
    pub chan_pos_252: Option<f64>,
    /// `close > sma200 AND sma50 > sma200`. A filter that CAPS the tier — not
    /// a score, and its absence is a value thesis, not a broken one.
    pub trend_ok: Option<bool>,
    /// `close/sma200 - 1`, clipped to ±0.5. Magnitude, for the tiebreak.
    pub trend_strength: Option<f64>,
    /// `close > dch_hi(55)` within the last 5 sessions.
    pub breakout_55_fresh: Option<bool>,
    /// `close > dch_hi(20)` within the last 5 sessions.
    pub breakout_20_fresh: Option<bool>,
    /// `volume / median(volume, 50)`. CONFIRMS a breakout; ranking on it is
    /// rejected outright (Lee & Swaminathan: high turnover predicts LOWER
    /// returns).
    pub rvol: Option<f64>,
    /// The 3-day form of [`Self::rvol`].
    pub rvol3: Option<f64>,
    /// `rvol >= 1.5 AND rvol3 >= 1.25`. Defined only on a breakout bar —
    /// `None` off a breakout means undefined, not false.
    pub vol_confirm: Option<bool>,
    /// `median(close * volume, 20)`. TRADABILITY, never alpha.
    pub dollar_volume_20d: Option<f64>,
    /// `atr20 / close`. The stop width this name would need.
    pub atr_pct: Option<f64>,
    /// `median(vol,20)/median(vol,250) > 2.0` — sustained elevated turnover is
    /// a REVERSAL flag, the opposite of a positive.
    pub reversal_risk: Option<bool>,
    /// True when the name is below the $5 filter the literature uses and only
    /// cleared the relaxed $1 floor (F0 §5.1 G3).
    pub relaxed_price_gate: bool,
    /// Within-tier ordering score, cross-sectional across THIS pack only.
    pub rank_score: Option<f64>,
}

impl CandidateEvidence {
    /// The block for a name whose indicators could not be computed from the
    /// bars that DID arrive — too little history, or a feed with no true highs
    /// and lows. Loud, and deliberately still `eligible`: the name cleared the
    /// loop gate, and the Financier judges it on what is stated.
    pub fn unavailable(reason: impl std::fmt::Display) -> Self {
        Self {
            status: format!("{INDICATORS_UNAVAILABLE}: {reason}"),
            eligible: true,
            ..Default::default()
        }
    }

    /// The block for a name whose bars never arrived at all. Separated from
    /// [`Self::unavailable`] because "the history is short" and "the feed was
    /// down" are different facts, and reporting the second as the first would
    /// be a claim about the security rather than about the network.
    pub fn feed_failed(reason: impl std::fmt::Display) -> Self {
        Self {
            status: format!("indicators unavailable — the daily bar feed failed: {reason}"),
            eligible: true,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloseCandidate {
    pub ticker: String,
    pub company_name: Option<String>,
    pub rank: Option<i64>,
    pub score: Option<f64>,
    pub confidence: Option<f64>,
    pub buy_window: Option<String>,
    pub reason: Option<String>,
    pub last: Option<f64>,
    pub loop_passed: bool,
    /// The indicator layer's read on this name (F0 §5). Never replaces
    /// `loop_passed`; see the module docs on the one-way rule.
    pub evidence: CandidateEvidence,
}

pub async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<(), String> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS finance_daily_picks (
            day              TEXT PRIMARY KEY,
            as_of            TEXT NOT NULL,
            ticker           TEXT,
            company_name     TEXT,
            why              TEXT NOT NULL,
            model            TEXT,
            candidate_count  INTEGER NOT NULL DEFAULT 0,
            created_at       TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn load_for_day(pool: &Pool<Sqlite>, day: &str) -> Result<Option<DailyPick>, String> {
    ensure_schema(pool).await?;
    let row = sqlx::query(
        "SELECT day, as_of, ticker, company_name, why, model, candidate_count
         FROM finance_daily_picks WHERE day = ?",
    )
    .bind(day)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(|r| DailyPick {
        day: r.get("day"),
        as_of: r.get("as_of"),
        ticker: r.get("ticker"),
        company_name: r.get("company_name"),
        why: r.get("why"),
        model: r.get("model"),
        candidate_count: r.get("candidate_count"),
    }))
}

pub async fn latest(pool: &Pool<Sqlite>) -> Result<Option<DailyPick>, String> {
    ensure_schema(pool).await?;
    let row = sqlx::query(
        "SELECT day, as_of, ticker, company_name, why, model, candidate_count
         FROM finance_daily_picks ORDER BY day DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(|r| DailyPick {
        day: r.get("day"),
        as_of: r.get("as_of"),
        ticker: r.get("ticker"),
        company_name: r.get("company_name"),
        why: r.get("why"),
        model: r.get("model"),
        candidate_count: r.get("candidate_count"),
    }))
}

pub async fn save(pool: &Pool<Sqlite>, pick: &DailyPick) -> Result<(), String> {
    ensure_schema(pool).await?;
    sqlx::query(
        "INSERT OR REPLACE INTO finance_daily_picks
            (day, as_of, ticker, company_name, why, model, candidate_count, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&pick.day)
    .bind(&pick.as_of)
    .bind(&pick.ticker)
    .bind(&pick.company_name)
    .bind(&pick.why)
    .bind(&pick.model)
    .bind(pick.candidate_count)
    .bind(&pick.as_of)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// `close > dch_hi(window)` on any of the last [`BREAKOUT_FRESH_SESSIONS`]
/// bars, each evaluated against ITS OWN prior window.
///
/// Re-running the channel per bar is the point: asking whether today's close
/// beats a channel drawn five days ago would let a name that has since sold
/// off keep claiming a breakout it no longer has.
fn breakout_fresh(bars: &[DailyBar], window: usize) -> Result<bool, ind::IndicatorError> {
    for back in 0..BREAKOUT_FRESH_SESSIONS {
        if bars.len() <= back {
            break;
        }
        let slice = &bars[..bars.len() - back];
        match ind::donchian_hi(slice, window) {
            Ok(hi) => {
                if slice[slice.len() - 1].close > hi {
                    return Ok(true);
                }
            }
            // The newest slice is the one that must be computable; an older
            // slice running out of history just ends the lookback early.
            Err(e) if back == 0 => return Err(e),
            Err(_) => break,
        }
    }
    Ok(false)
}

/// F0 §5.1 G2's `intended_position_$` — the dollars actually being put to work.
///
/// F0 ties the liquidity floor to position size and forbids picking a number:
/// "threshold tied to intended position size, not a magic number". The only
/// honest source on this machine is the user's own journal, so this is the
/// median notional (`entry_price * shares`) of the trades Picker has recorded.
/// An empty journal yields `None`, and the gate is then **not applied** —
/// stating that in the evidence is honest; inventing a threshold to have one
/// is not.
pub fn intended_position_usd(trades: &[picker::TradeRow]) -> Option<f64> {
    let mut notional: Vec<f64> = trades
        .iter()
        .map(|t| t.entry_price * t.shares as f64)
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect();
    if notional.is_empty() {
        return None;
    }
    notional.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = notional.len();
    Some(if n % 2 == 1 {
        notional[n / 2]
    } else {
        (notional[n / 2 - 1] + notional[n / 2]) / 2.0
    })
}

/// F0 §5.2's tier, given the reads it depends on. Ordinal, never summed.
fn tier_for(
    trend_ok: bool,
    chan_pos: f64,
    breakout_55_fresh: bool,
    vol_confirm: Option<bool>,
    reversal_risk: bool,
) -> &'static str {
    if trend_ok
        && chan_pos >= CHAN_POS_TIER_A
        && breakout_55_fresh
        && vol_confirm == Some(true)
        && !reversal_risk
    {
        "A"
    } else if trend_ok && chan_pos >= CHAN_POS_TIER_B {
        "B"
    } else if trend_ok != (chan_pos >= CHAN_POS_TIER_B) {
        "C"
    } else {
        "D"
    }
}

/// Build one candidate's evidence block from its adjusted daily bars.
///
/// Pure and total: every indicator that cannot be computed comes back `None`
/// with the reason folded into `status`, and the whole block degrades to
/// [`CandidateEvidence::unavailable`] rather than reporting a number measured
/// over a window it does not name.
pub fn evidence_from_bars(
    bars: &[DailyBar],
    intended_position_usd: Option<f64>,
) -> CandidateEvidence {
    let chan_pos = match ind::chan_pos_252(bars) {
        Ok(v) => v,
        Err(e) => return CandidateEvidence::unavailable(e),
    };
    let close = bars[bars.len() - 1].close;
    let trend_ok = ind::trend_ok(bars).ok();
    let trend_strength = ind::trend_strength(bars).ok();
    let atr_pct = ind::atr_pct(bars).ok();
    let dv20 = ind::dollar_volume_20d_median(bars).ok();
    let rvol = ind::rvol(bars).ok();
    let rvol3 = ind::rvol3(bars).ok();
    let reversal_risk = ind::reversal_risk(bars).ok();
    let breakout_55_fresh = breakout_fresh(bars, ind::DONCHIAN_LONG).ok();
    let breakout_20_fresh = breakout_fresh(bars, ind::DONCHIAN_SHORT).ok();

    // F0 §3.2/§5.2: confirmation is DEFINED ONLY on a breakout bar. Off one it
    // is undefined, which is not the same as unconfirmed.
    let on_breakout = breakout_55_fresh == Some(true) || breakout_20_fresh == Some(true);
    let vol_confirm = match (on_breakout, rvol, rvol3) {
        (true, Some(r), Some(r3)) => Some(r >= ind::RVOL_CONFIRM && r3 >= ind::RVOL3_CONFIRM),
        _ => None,
    };

    // ── Eligibility gate (F0 §5.1), evaluated before any ranking ──
    let mut vetoes = Vec::new();
    if close < MIN_PRICE {
        vetoes.push(format!(
            "close {close:.2} is below the {MIN_PRICE:.2} floor — sub-dollar names trade in a \
             delisting and spread regime these rules were never measured in (F0 §5.1 G3)"
        ));
    }
    if let Some(a) = atr_pct {
        if a > MAX_ATR_PCT {
            vetoes.push(format!(
                "atr_pct {:.3} is above {MAX_ATR_PCT:.2} — a 3xATR stop would sit {:.0}% wide \
                 (F0 §5.1 G4)",
                a,
                a * ind::CHANDELIER_K * 100.0
            ));
        }
    }
    match (dv20, intended_position_usd) {
        (Some(dv), Some(size)) if dv < DOLLAR_VOLUME_MULTIPLE * size => {
            vetoes.push(format!(
                "dv20 {dv:.0} is below {DOLLAR_VOLUME_MULTIPLE:.0}x the {size:.0} position this \
                 book actually takes — the round trip costs more than any plausible edge \
                 (F0 §5.1 G2)"
            ));
        }
        _ => {}
    }

    let eligible = vetoes.is_empty();
    let tier = match (eligible, trend_ok, reversal_risk) {
        (true, Some(t), Some(rr)) => Some(
            tier_for(
                t,
                chan_pos,
                breakout_55_fresh == Some(true),
                vol_confirm,
                rr,
            )
            .to_string(),
        ),
        _ => None,
    };

    let status = if !eligible {
        format!("ineligible — {}", vetoes.join("; "))
    } else if let Some(t) = &tier {
        format!(
            "tier {t} — trend_ok {}, chan_pos_252 {:.3}",
            trend_ok
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
            chan_pos
        )
    } else {
        // Channel position computed but the trend or turnover window did not,
        // so there is no tier. Not the INDICATORS_UNAVAILABLE case: some of the
        // numbers below ARE real, and claiming otherwise would be as misleading
        // in one direction as a shortened window is in the other.
        "partially available — no tier assigned: the trend or turnover window did not complete"
            .to_string()
    };

    CandidateEvidence {
        status,
        tier,
        eligible,
        vetoes,
        chan_pos_252: Some(chan_pos),
        trend_ok,
        trend_strength,
        breakout_55_fresh,
        breakout_20_fresh,
        rvol,
        rvol3,
        vol_confirm,
        dollar_volume_20d: dv20,
        atr_pct,
        reversal_risk,
        relaxed_price_gate: close < LITERATURE_PRICE_FLOOR,
        rank_score: None,
    }
}

/// Cross-sectional z-score. A single point, or a pack with no spread, scores
/// zero — a lone candidate is not "one standard deviation good".
fn z_scores(values: &[Option<f64>]) -> Vec<f64> {
    let present: Vec<f64> = values.iter().flatten().copied().collect();
    if present.len() < 2 {
        return vec![0.0; values.len()];
    }
    let mean = present.iter().sum::<f64>() / present.len() as f64;
    let var = present.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / present.len() as f64;
    let sd = var.sqrt();
    if !(sd.is_finite() && sd > 0.0) {
        return vec![0.0; values.len()];
    }
    values
        .iter()
        .map(|v| v.map(|x| (x - mean) / sd).unwrap_or(0.0))
        .collect()
}

/// Rank tier order: A before B before C before D, and an untiered name after
/// all of them. Untiered is not Tier E — it is "not rated", and the sort keeps
/// it out of the way without pretending it lost on merit.
fn tier_ordinal(tier: Option<&str>) -> u8 {
    match tier {
        Some("A") => 0,
        Some("B") => 1,
        Some("C") => 2,
        Some("D") => 3,
        _ => 4,
    }
}

/// Order the pack the Financier will read: veto first, then tier, then the
/// within-tier tiebreak.
///
/// **The one-way rule.** Anything that did not pass the loop gate is dropped
/// here, unconditionally and before any indicator is read. The indicator layer
/// exists to remove and to reorder; it has no path by which a name the
/// significance gate rejected can reach the judge, however good its channel
/// position looks. Enforcing that here — rather than trusting the caller never
/// to build such a candidate — is what makes the rule structural.
///
/// Within a tier: `0.50*z(chan_pos_252) + 0.30*z(trend_strength) +
/// 0.20*z(log dv20)` (F0 §5.2). The dv20 term is a **tradability** tiebreak,
/// not an alpha term.
pub fn rank_candidates(candidates: Vec<CloseCandidate>) -> Vec<CloseCandidate> {
    let mut kept: Vec<CloseCandidate> = candidates.into_iter().filter(|c| c.loop_passed).collect();

    let chan: Vec<Option<f64>> = kept.iter().map(|c| c.evidence.chan_pos_252).collect();
    let trend: Vec<Option<f64>> = kept.iter().map(|c| c.evidence.trend_strength).collect();
    let dv: Vec<Option<f64>> = kept
        .iter()
        .map(|c| {
            c.evidence
                .dollar_volume_20d
                .filter(|v| *v > 0.0)
                .map(f64::ln)
        })
        .collect();
    let (zc, zt, zd) = (z_scores(&chan), z_scores(&trend), z_scores(&dv));
    for (i, c) in kept.iter_mut().enumerate() {
        c.evidence.rank_score = c
            .evidence
            .chan_pos_252
            .map(|_| 0.50 * zc[i] + 0.30 * zt[i] + 0.20 * zd[i]);
    }

    kept.sort_by(|a, b| {
        let key = |c: &CloseCandidate| {
            (
                !c.evidence.eligible,
                tier_ordinal(c.evidence.tier.as_deref()),
            )
        };
        key(a).cmp(&key(b)).then_with(|| {
            b.evidence
                .rank_score
                .unwrap_or(f64::NEG_INFINITY)
                .partial_cmp(&a.evidence.rank_score.unwrap_or(f64::NEG_INFINITY))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    kept
}

/// Picker names that survive Yahoo + the loop gate, each carrying its
/// indicator evidence, ordered by [`rank_candidates`].
///
/// A name that fails the loop gate drops out here as it always has — a missing
/// series is not a silent pass. A name whose *indicators* cannot be computed
/// does NOT drop out: it reaches the judge with an evidence block that says so.
pub async fn surviving_candidates() -> Result<Vec<CloseCandidate>, String> {
    let raw = picker::top_picks().await?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    // The liquidity floor is a multiple of the size this book actually takes,
    // read from the trade journal rather than chosen (F0 §5.1 G2). A journal
    // we cannot read means the gate is not applied, and the evidence says so.
    let position_size = match picker::trades().await {
        Ok(raw) => {
            let rows: Vec<picker::TradeRow> =
                raw.iter().filter_map(picker::parse_trade_row).collect();
            intended_position_usd(&rows)
        }
        Err(_) => None,
    };
    let batch = raw.len().min(MAX_CANDIDATES);
    let mut out = Vec::new();
    for v in raw.into_iter().take(MAX_CANDIDATES) {
        let Some(ticker) = v
            .get("ticker")
            .or_else(|| v.get("symbol"))
            .and_then(|s| s.as_str())
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let closes = match market_data::daily_closes(&ticker, "1y").await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let gate = pick_loop::validate_closes(&closes, batch);
        if !gate.passed {
            continue;
        }
        let last = market_data::quote(&ticker).await.ok().and_then(|q| q.price);
        // A second fetch, deliberately. The gate above runs on `daily_closes`
        // over "1y" and the indicators need true highs, lows and volume over
        // 300 bars — and feeding the gate a different window would silently
        // change which names pass it, which is not a change this layer is
        // allowed to make. 300 because chan_pos_252 needs 252 bars BEFORE
        // today and "1y" would fall short on the first holiday (F0 §6).
        let evidence = match market_data::daily_bars(&ticker, market_data::DEFAULT_BARS_RANGE).await
        {
            Ok(bars) => evidence_from_bars(&bars, position_size),
            Err(e) => CandidateEvidence::feed_failed(e),
        };
        out.push(CloseCandidate {
            ticker,
            company_name: v
                .get("company_name")
                .or_else(|| v.get("name"))
                .and_then(|s| s.as_str())
                .map(str::to_string),
            rank: v.get("rank").and_then(|n| n.as_i64()),
            score: v
                .get("total_score")
                .or_else(|| v.get("score"))
                .and_then(|n| n.as_f64()),
            confidence: v
                .get("confidence")
                .or_else(|| v.get("conv"))
                .and_then(|n| n.as_f64()),
            buy_window: v
                .get("buy_window")
                .or_else(|| v.get("buyWindow"))
                .and_then(|s| s.as_str())
                .map(str::to_string),
            reason: v
                .get("reason")
                .or_else(|| v.get("thesis"))
                .and_then(|s| s.as_str())
                .map(str::to_string),
            last,
            loop_passed: true,
            evidence,
        });
    }
    Ok(rank_candidates(out))
}

pub fn none_pick(day: &str, why: impl Into<String>, candidates: usize) -> DailyPick {
    DailyPick {
        day: day.to_string(),
        as_of: Utc::now().to_rfc3339(),
        ticker: None,
        company_name: None,
        why: why.into(),
        model: None,
        candidate_count: candidates as i64,
    }
}

/// The judge's standing instructions. Unchanged in substance since before the
/// indicator layer: at most one name, from the list, or none.
pub const JUDGE_SYSTEM_PROMPT: &str = "You are The Financier. You may name AT MOST one ticker \
     from CANDIDATES as tomorrow's pick, or none. A pick is a hypothesis, not an order and not \
     a size. NEVER invent a ticker that is not in the list. NEVER invent signs that are not in \
     the supplied fields. If nothing is good enough, pick is null. Reply JSON only: \
     {\"pick\": \"TICKER\" or null, \"why\": \"one paragraph\"}.";

/// How to read the `evidence` block — the F0 §5 combination rules, in words.
///
/// This exists because every one of these numbers means something different
/// from what its name suggests to a reader who has met technical indicators
/// before. `dollarVolume20d` is a tradability floor, and an LLM left to its
/// own priors will read it as "more volume is better", which F0 §3.3 says is
/// false. `reversalRisk` is high turnover, which the same priors read as
/// enthusiasm. `rvol` confirms a breakout and ranks nothing. Stating the
/// semantics is not politeness; it is the difference between the evidence pack
/// helping and actively misleading the judge.
pub const EVIDENCE_SEMANTICS: &str = "Each candidate carries an `evidence` block. Read it in \
     this order, and do not re-derive it.\n\
     1. LIQUIDITY AND PRICE GATE FIRST. `eligible: false` means a hard veto fired (listed in \
     `vetoes`) — that name CANNOT be tomorrow's pick, whatever else it shows. `dollarVolume20d` \
     is the 20-day median of close x volume: it is a TRADABILITY floor, NOT a quality score. \
     More dollar volume is not better; enough dollar volume is merely permitted.\n\
     2. `trendOk` CAPS THE TIER. It is `close > SMA200 AND SMA50 > SMA200`. A name below its \
     SMA200 may still be a good value thesis — it is simply not a trend thesis, and it cannot \
     reach the top tier.\n\
     3. `chanPos252` RANKS WITHIN A TIER. It is where today's close sits inside the prior \
     252-day high/low range, 0 to 1, and values above 1.0 mean a genuine new high. This is the \
     one field with cross-sectional return evidence behind it.\n\
     4. `rvol` CONFIRMS A BREAKOUT AND NOTHING ELSE. `volConfirm` is only defined when \
     `breakout20Fresh` or `breakout55Fresh` is true; `null` there means undefined, not \
     unconfirmed. High volume on its own is not a reason to buy.\n\
     5. `reversalRisk: true` IS A WARNING, not enthusiasm: sustained elevated turnover predicts \
     LOWER forward returns in exactly this kind of name.\n\
     `tier` is A (best) to D, already computed; the list arrives in rank order. `tier: null` \
     with a `status` saying indicators are unavailable means the history was too short to \
     compute them — judge that name on the scanner's own case, and do NOT assume the missing \
     numbers would have been good or bad. Nothing in this block is a probability, and none of \
     it overrides the loop gate that every listed name already passed.";

/// Opus (or the configured Orchestrate role if it is Opus) chooses at most
/// one candidate. Invented tickers become none.
pub async fn judge_with_opus(
    day: &str,
    candidates: &[CloseCandidate],
) -> Result<DailyPick, String> {
    if candidates.is_empty() {
        return Ok(none_pick(
            day,
            "No scanner names cleared the loop gate. No pick for tomorrow.",
            0,
        ));
    }
    let (provider_name, model_name) = opus_model()?;
    let provider =
        crate::providers::create_with_named_model(&provider_name, &model_name, Vec::new())
            .await
            .map_err(|e| format!("Opus is not available ({e}). No pick invented."))?;
    let system = format!("{JUDGE_SYSTEM_PROMPT}\n\n{EVIDENCE_SEMANTICS}");
    let user = Message::user().with_text(format!(
        "Session day {day}. CANDIDATES:\n{}",
        serde_json::to_string_pretty(candidates).unwrap_or_default()
    ));
    let manager = std::sync::Arc::new(crate::session::SessionManager::instance());
    let session = AccountedFastCompletion::ensure_background_session(
        std::sync::Arc::clone(&manager),
        "financier-close",
    )
    .await
    .map_err(|e| format!("Financier session unavailable ({e}). No pick invented."))?;
    let (response, _usage) = AccountedFastCompletion::complete_accounted(
        manager,
        session,
        provider,
        &system,
        std::slice::from_ref(&user),
        &[],
        false,
    )
    .await
    .map_err(|e| format!("Opus did not answer ({e}). No pick invented."))?;
    Ok(parse_judgment(
        day,
        &response.as_concat_text(),
        candidates,
        &format!("{provider_name}/{model_name}"),
    ))
}

fn opus_model() -> Result<(String, String), String> {
    if let Some(mapped) = role_map::role_model(WorkflowRole::Orchestrate) {
        if mapped.model.to_ascii_lowercase().contains("opus") {
            return Ok((mapped.provider, mapped.model));
        }
    }
    Ok((OPUS_PROVIDER.to_string(), OPUS_MODEL.to_string()))
}

/// Turn Opus's reply into the day's pick.
///
/// Two refusals, not one. A ticker that was never in the list is refused as it
/// always has been; a ticker whose evidence carries a hard eligibility veto is
/// ALSO refused, even though it was shown. Vetoed names stay in the pack so the
/// judge can see what was ruled out and why — the prompt is not a place to hide
/// information — but a veto that only asked the model nicely would not be a
/// veto, so it is enforced here in code.
pub fn parse_judgment(
    day: &str,
    text: &str,
    candidates: &[CloseCandidate],
    model: &str,
) -> DailyPick {
    let allowed: Vec<&str> = candidates
        .iter()
        .filter(|c| c.evidence.eligible)
        .map(|c| c.ticker.as_str())
        .collect();
    let parsed = extract_json(text).map(|v| {
        let pick = match v.get("pick") {
            Some(serde_json::Value::Null) | None => None,
            Some(serde_json::Value::String(s)) => {
                let t = s.trim().to_uppercase();
                if t.is_empty() || t == "NULL" || t == "NONE" {
                    None
                } else {
                    Some(t)
                }
            }
            _ => None,
        };
        let why = v
            .get("why")
            .and_then(|s| s.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("No paragraph.")
            .to_string();
        (pick, why)
    });
    let Some((pick, why)) = parsed else {
        return none_pick(
            day,
            "Opus did not return a usable judgment. No pick invented.",
            candidates.len(),
        );
    };
    match pick {
        Some(ticker) if allowed.iter().any(|a| *a == ticker) => {
            let company = candidates
                .iter()
                .find(|c| c.ticker == ticker)
                .and_then(|c| c.company_name.clone());
            DailyPick {
                day: day.to_string(),
                as_of: Utc::now().to_rfc3339(),
                ticker: Some(ticker),
                company_name: company,
                why,
                model: Some(model.to_string()),
                candidate_count: candidates.len() as i64,
            }
        }
        Some(ticker) => {
            let vetoed = candidates
                .iter()
                .find(|c| c.ticker == ticker && !c.evidence.eligible);
            match vetoed {
                Some(c) => none_pick(
                    day,
                    format!(
                        "Opus named {ticker}, which the eligibility gate had already vetoed: {}. \
                         No pick invented.",
                        c.evidence.vetoes.join("; ")
                    ),
                    candidates.len(),
                ),
                None => none_pick(
                    day,
                    "Opus named a ticker that was not in the scanner list. No pick invented.",
                    candidates.len(),
                ),
            }
        }
        None => DailyPick {
            day: day.to_string(),
            as_of: Utc::now().to_rfc3339(),
            ticker: None,
            company_name: None,
            why,
            model: Some(model.to_string()),
            candidate_count: candidates.len() as i64,
        },
    }
}

fn extract_json(text: &str) -> Option<serde_json::Value> {
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    serde_json::from_str(text.get(start..=end)?).ok()
}

pub fn notify_copy(pick: &DailyPick) -> (String, String) {
    match pick.ticker.as_deref() {
        Some(ticker) => (
            "The Financier · tomorrow".into(),
            format!("{ticker} — {}", pick.why),
        ),
        None => ("The Financier · no pick tomorrow".into(), pick.why.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn financier_judge_cannot_bypass_shared_paid_dispatch_boundary() {
        let source = include_str!("financier_close.rs");
        let direct_call = [".", "complete("].concat();
        assert!(source.contains("complete_accounted"));
        assert!(!source.contains(&direct_call));
    }

    fn cand(ticker: &str) -> CloseCandidate {
        CloseCandidate {
            ticker: ticker.into(),
            company_name: Some(ticker.into()),
            rank: Some(1),
            score: Some(1.0),
            confidence: Some(0.6),
            buy_window: None,
            reason: Some("scanner reason".into()),
            last: Some(10.0),
            loop_passed: true,
            evidence: CandidateEvidence {
                status: "tier B".into(),
                tier: Some("B".into()),
                eligible: true,
                ..Default::default()
            },
        }
    }

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

    /// A rising series: `n` bars climbing by `step`, high/low straddling the
    /// close by 1. With `step > 1` every bar closes above the prior bar's HIGH,
    /// so the name is in a continuous Donchian breakout — which is what a
    /// Tier A fixture needs.
    fn ramp(n: usize, base: f64, step: f64, volume: u64) -> Vec<DailyBar> {
        (0..n)
            .map(|i| {
                let c = base + step * i as f64;
                bar(c + 1.0, c - 1.0, c, volume)
            })
            .collect()
    }

    /// The Tier A fixture: a 300-bar climb whose last three sessions trade at
    /// twice normal volume, so `rvol` (2.0) and `rvol3` (2.0) both clear their
    /// confirmation thresholds while the 20-vs-250 turnover medians stay flat
    /// and `reversal_risk` stays false.
    fn tier_a_bars() -> Vec<DailyBar> {
        let mut bars = ramp(300, 100.0, 2.0, 1_000);
        for b in bars.iter_mut().rev().take(3) {
            b.volume = 2_000;
        }
        bars
    }

    // ── the one-way rule ──────────────────────────────────────────────────

    /// The single most important test in this file. The indicator layer may
    /// remove a survivor or reorder survivors; it may NEVER put a loop-gate
    /// failure in front of the judge, however good its numbers look.
    #[test]
    fn indicators_never_promote_a_loop_gate_failure() {
        let mut failer = cand("GATEFAIL");
        failer.loop_passed = false;
        // Perfect evidence: top tier, top of its 252-day range, deep liquidity.
        failer.evidence = CandidateEvidence {
            status: "tier A".into(),
            tier: Some("A".into()),
            eligible: true,
            chan_pos_252: Some(1.4),
            trend_ok: Some(true),
            trend_strength: Some(0.5),
            dollar_volume_20d: Some(50_000_000.0),
            ..Default::default()
        };
        // A mediocre name that DID pass.
        let mut passer = cand("PASSED");
        passer.evidence = CandidateEvidence {
            status: "tier D".into(),
            tier: Some("D".into()),
            eligible: true,
            chan_pos_252: Some(0.10),
            trend_ok: Some(false),
            trend_strength: Some(-0.3),
            dollar_volume_20d: Some(90_000.0),
            ..Default::default()
        };

        let ranked = rank_candidates(vec![failer, passer]);
        assert_eq!(
            ranked.iter().map(|c| c.ticker.as_str()).collect::<Vec<_>>(),
            vec!["PASSED"],
            "a loop-gate failure must not survive ranking, and must not be reordered to the top"
        );
    }

    #[test]
    fn ranking_can_only_ever_return_a_subset_of_what_it_was_given() {
        let input = vec![cand("AAA"), cand("BBB"), cand("CCC")];
        let names: Vec<String> = input.iter().map(|c| c.ticker.clone()).collect();
        let ranked = rank_candidates(input);
        assert!(ranked.len() <= names.len());
        for c in &ranked {
            assert!(names.contains(&c.ticker), "invented {}", c.ticker);
        }
    }

    // ── evidence, and the loud absence of it ──────────────────────────────

    #[test]
    fn short_history_says_so_instead_of_shortening_the_window() {
        // 120 bars is a real series and a useless one: chan_pos_252 needs 253.
        let bars = ramp(120, 10.0, 0.5, 1_000);
        let e = evidence_from_bars(&bars, Some(1_000.0));
        assert!(
            e.status.starts_with(INDICATORS_UNAVAILABLE),
            "status was {:?}",
            e.status
        );
        assert!(
            e.status.contains("252"),
            "the reason names the window it needed: {:?}",
            e.status
        );
        assert_eq!(e.chan_pos_252, None);
        assert_eq!(e.tier, None);
        assert!(
            e.eligible,
            "a name whose indicators are unavailable is judged on what is stated, not vetoed"
        );
    }

    #[test]
    fn a_close_only_feed_is_reported_not_computed_over() {
        // High and low equal the close on every bar — Donchian and ATR are
        // undefined here, and substituting closes would silently change what
        // they measure.
        let bars: Vec<DailyBar> = (0..300)
            .map(|i| {
                let c = 10.0 + i as f64;
                bar(c, c, c, 1_000)
            })
            .collect();
        let e = evidence_from_bars(&bars, Some(1_000.0));
        assert!(
            e.status.starts_with(INDICATORS_UNAVAILABLE),
            "{:?}",
            e.status
        );
        assert!(e.status.contains("high"), "{:?}", e.status);
        assert_eq!(e.chan_pos_252, None);
        assert!(e.eligible);
    }

    #[test]
    fn a_dead_feed_is_not_reported_as_a_short_history() {
        let e = CandidateEvidence::feed_failed("could not fetch daily bars: timed out");
        assert!(e.status.contains("feed failed"), "{:?}", e.status);
        assert!(
            !e.status.starts_with(INDICATORS_UNAVAILABLE),
            "a network failure is not a claim about the security's history"
        );
        assert!(e.eligible);
    }

    // ── the eligibility gate ──────────────────────────────────────────────

    #[test]
    fn a_sub_dollar_close_is_vetoed_and_a_cheap_one_is_only_flagged() {
        let mut bars = tier_a_bars();
        let scale = 0.9 / bars.last().unwrap().close;
        for b in bars.iter_mut() {
            b.high *= scale;
            b.low *= scale;
            b.close *= scale;
        }
        let e = evidence_from_bars(&bars, None);
        assert!(!e.eligible, "0.90 is under the 1.00 floor");
        assert!(
            e.vetoes.iter().any(|v| v.contains("below the 1.00 floor")),
            "{:?}",
            e.vetoes
        );
        assert_eq!(e.tier, None, "a vetoed name is not tiered");

        // A $4 name clears the relaxed floor but is RECORDED as having done so,
        // because the literature screens at $5 and F5 must be able to separate
        // the two groups.
        let mut cheap = tier_a_bars();
        let scale = 4.0 / cheap.last().unwrap().close;
        for b in cheap.iter_mut() {
            b.high *= scale;
            b.low *= scale;
            b.close *= scale;
        }
        let e = evidence_from_bars(&cheap, None);
        assert!(e.eligible);
        assert!(
            e.relaxed_price_gate,
            "a $4 name passed on the relaxed threshold"
        );
    }

    #[test]
    fn the_liquidity_floor_is_a_multiple_of_the_size_this_book_actually_takes() {
        let bars = tier_a_bars();
        let dv = ind::dollar_volume_20d_median(&bars).unwrap();
        // dv20 is about 690k here; a 1k position needs 25k and passes.
        assert!(evidence_from_bars(&bars, Some(1_000.0)).eligible);
        // A position 1/25th of dv20 is exactly the boundary; anything larger
        // fails, because the round trip starts to cost more than the edge.
        let too_big = dv / DOLLAR_VOLUME_MULTIPLE * 1.01;
        let e = evidence_from_bars(&bars, Some(too_big));
        assert!(!e.eligible);
        assert!(
            e.vetoes.iter().any(|v| v.contains("dv20")),
            "{:?}",
            e.vetoes
        );
        // With no journal there is no honest threshold, so the gate does not
        // fire — the number is still reported.
        let e = evidence_from_bars(&bars, None);
        assert!(e.eligible);
        assert!(e.dollar_volume_20d.is_some());
    }

    #[test]
    fn intended_position_size_is_read_from_the_journal_or_not_invented() {
        assert_eq!(intended_position_usd(&[]), None);
        let rows = vec![
            trade("AAA", 10.0, 100), // 1000
            trade("BBB", 5.0, 100),  // 500
            trade("CCC", 30.0, 100), // 3000
        ];
        assert_eq!(intended_position_usd(&rows), Some(1_000.0));
    }

    fn trade(ticker: &str, entry_price: f64, shares: i64) -> picker::TradeRow {
        picker::TradeRow {
            id: ticker.into(),
            ticker: ticker.into(),
            company_name: ticker.into(),
            entry_date: "2026-08-01".into(),
            entry_price,
            shares,
            exit_date: None,
            exit_price: None,
            notes: None,
        }
    }

    #[test]
    fn a_veto_is_enforced_in_code_not_merely_asked_for_in_the_prompt() {
        let mut vetoed = cand("THIN");
        vetoed.evidence = CandidateEvidence {
            status: "ineligible".into(),
            tier: None,
            eligible: false,
            vetoes: vec!["dv20 900 is below 25x the 1000 position this book actually takes".into()],
            ..Default::default()
        };
        let got = parse_judgment(
            "2026-08-31",
            r#"{"pick":"THIN","why":"It looks explosive."}"#,
            &[vetoed, cand("SHOP")],
            "anthropic/claude-opus-4-8",
        );
        assert!(got.ticker.is_none(), "a vetoed name cannot become the pick");
        assert!(got.why.contains("already vetoed"), "{}", got.why);
        assert!(
            got.why.contains("dv20"),
            "the refusal repeats the reason: {}",
            got.why
        );
    }

    // ── tiers and ordering ────────────────────────────────────────────────

    #[test]
    fn tiers_are_ordinal_and_follow_the_locked_combination_spec() {
        // A needs every condition at once.
        assert_eq!(tier_for(true, 0.85, true, Some(true), false), "A");
        // …and loses the top tier to any one of them.
        assert_eq!(
            tier_for(true, 0.85, true, Some(true), true),
            "B",
            "reversal risk"
        );
        assert_eq!(
            tier_for(true, 0.85, false, Some(true), false),
            "B",
            "stale breakout"
        );
        assert_eq!(
            tier_for(true, 0.85, true, None, false),
            "B",
            "confirmation undefined"
        );
        assert_eq!(
            tier_for(true, 0.85, true, Some(false), false),
            "B",
            "unconfirmed"
        );
        assert_eq!(
            tier_for(false, 0.95, true, Some(true), false),
            "C",
            "no trend caps it at C"
        );
        // C is the exclusive-or: exactly one of trend or channel position.
        assert_eq!(tier_for(true, 0.30, false, None, false), "C", "trend only");
        assert_eq!(
            tier_for(false, 0.75, false, None, false),
            "C",
            "channel only"
        );
        assert_eq!(tier_for(false, 0.30, false, None, false), "D", "neither");
        // The boundary itself is inclusive, as written.
        assert_eq!(tier_for(true, CHAN_POS_TIER_B, false, None, false), "B");
    }

    #[test]
    fn the_pack_arrives_tier_first_then_channel_position_then_vetoes_last() {
        let mk = |t: &str, tier: &str, chan: f64, eligible: bool| {
            let mut c = cand(t);
            c.evidence = CandidateEvidence {
                status: tier.into(),
                tier: eligible.then(|| tier.to_string()),
                eligible,
                chan_pos_252: Some(chan),
                trend_strength: Some(0.1),
                dollar_volume_20d: Some(500_000.0),
                ..Default::default()
            };
            c
        };
        let ranked = rank_candidates(vec![
            mk("DEE", "D", 0.10, true),
            mk("BEE_LOW", "B", 0.62, true),
            mk("VETOED", "A", 0.99, false),
            mk("AYE", "A", 0.90, true),
            mk("BEE_HIGH", "B", 0.75, true),
        ]);
        assert_eq!(
            ranked.iter().map(|c| c.ticker.as_str()).collect::<Vec<_>>(),
            vec!["AYE", "BEE_HIGH", "BEE_LOW", "DEE", "VETOED"],
            "tier decides across tiers; channel position decides within one; a veto sorts last"
        );
        assert!(ranked[0].evidence.rank_score.is_some());
    }

    #[test]
    fn an_unrated_name_sorts_after_the_rated_ones_without_being_called_worst() {
        let mut rated = cand("RATED");
        rated.evidence = CandidateEvidence {
            tier: Some("D".into()),
            eligible: true,
            chan_pos_252: Some(0.05),
            ..Default::default()
        };
        let unrated = {
            let mut c = cand("UNRATED");
            c.evidence = CandidateEvidence::unavailable("too few bars");
            c
        };
        let ranked = rank_candidates(vec![unrated, rated]);
        assert_eq!(
            ranked.iter().map(|c| c.ticker.as_str()).collect::<Vec<_>>(),
            vec!["RATED", "UNRATED"]
        );
        assert_eq!(
            ranked[1].evidence.tier, None,
            "unrated is not Tier E, and must not be scored as if it lost on merit"
        );
        assert_eq!(ranked[1].evidence.rank_score, None);
    }

    // ── breakout freshness ────────────────────────────────────────────────

    #[test]
    fn a_breakout_stays_fresh_for_five_sessions_and_re_derives_its_channel_daily() {
        // Flat for 100 bars, then one bar that clears the channel, then quiet
        // days BELOW that channel high.
        let mut bars: Vec<DailyBar> = (0..100).map(|_| bar(11.0, 9.0, 10.0, 1_000)).collect();
        bars.push(bar(21.0, 19.0, 20.0, 1_000)); // the breakout bar
        assert!(
            breakout_fresh(&bars, 20).unwrap(),
            "the breakout day itself"
        );

        for _ in 0..4 {
            bars.push(bar(13.0, 11.0, 12.0, 1_000));
        }
        assert!(
            breakout_fresh(&bars, 20).unwrap(),
            "four sessions later it is still inside the 5-session window"
        );
        bars.push(bar(13.0, 11.0, 12.0, 1_000));
        assert!(
            !breakout_fresh(&bars, 20).unwrap(),
            "six sessions later the breakout has expired — it is not a standing property"
        );
    }

    // ── one whole evidence block ──────────────────────────────────────────

    /// The block the Financier actually reads, on a hand-built series whose
    /// every number is checkable by eye. Printed by `--nocapture` so the exact
    /// bytes that reach the prompt can be reviewed rather than described.
    #[test]
    fn a_whole_evidence_block_for_one_candidate() {
        let bars = tier_a_bars();
        let e = evidence_from_bars(&bars, Some(1_000.0));
        let mut c = cand("RAMP");
        c.last = Some(bars.last().unwrap().close);
        c.evidence = e;
        let ranked = rank_candidates(vec![c]);
        let json = serde_json::to_string_pretty(&ranked[0]).unwrap();
        println!("{json}");

        let e = &ranked[0].evidence;
        assert_eq!(e.tier.as_deref(), Some("A"));
        assert_eq!(e.trend_ok, Some(true));
        assert_eq!(e.breakout_55_fresh, Some(true));
        assert_eq!(e.vol_confirm, Some(true));
        assert_eq!(e.reversal_risk, Some(false));
        assert!(!e.relaxed_price_gate);
        // 300 bars climbing by 2 from 100: today's close is 698, and every
        // prior close is lower, so the channel position is a new high.
        assert_eq!(bars.last().unwrap().close, 698.0);
        assert!(e.chan_pos_252.unwrap() > 1.0, "{:?}", e.chan_pos_252);
        // rvol: today's 2000 against a prior-50 median of 1000.
        assert_eq!(e.rvol, Some(2.0));
        assert_eq!(e.rvol3, Some(2.0));
        // A single candidate has no cross-section, so its z-scores are zero.
        assert_eq!(e.rank_score, Some(0.0));
    }

    #[test]
    fn the_prompt_states_what_each_number_means_before_the_judge_guesses() {
        // Every field the semantics block promises to explain must actually be
        // named in it — a legend that drifts from the payload is worse than no
        // legend, because the judge trusts it.
        for field in [
            "dollarVolume20d",
            "trendOk",
            "chanPos252",
            "rvol",
            "volConfirm",
            "reversalRisk",
            "breakout55Fresh",
            "eligible",
        ] {
            assert!(
                EVIDENCE_SEMANTICS.contains(field),
                "the evidence legend never explains {field}"
            );
        }
        // And the three readings an LLM's priors get backwards.
        assert!(EVIDENCE_SEMANTICS.contains("TRADABILITY floor, NOT a quality score"));
        assert!(EVIDENCE_SEMANTICS.contains("CAPS THE TIER"));
        assert!(EVIDENCE_SEMANTICS.contains("not unconfirmed"));
    }

    #[test]
    fn invented_ticker_is_refused() {
        let got = parse_judgment(
            "2026-08-24",
            r#"{"pick":"FAKE","why":"I made this up"}"#,
            &[cand("SHOP")],
            "anthropic/claude-opus-4-8",
        );
        assert!(got.ticker.is_none());
        assert!(got.why.contains("not in the scanner list"));
    }

    #[test]
    fn listed_ticker_is_kept() {
        let got = parse_judgment(
            "2026-08-24",
            r#"{"pick":"shop","why":"Loop gate held and the scanner's window is tomorrow."}"#,
            &[cand("SHOP"), cand("ENB")],
            "anthropic/claude-opus-4-8",
        );
        assert_eq!(got.ticker.as_deref(), Some("SHOP"));
        assert!(got.why.contains("Loop gate"));
    }

    #[test]
    fn null_pick_is_honest_none() {
        let got = parse_judgment(
            "2026-08-24",
            r#"{"pick":null,"why":"Both names look stretched into the close."}"#,
            &[cand("SHOP")],
            "anthropic/claude-opus-4-8",
        );
        assert!(got.ticker.is_none());
        assert!(got.why.contains("stretched"));
    }

    #[test]
    fn unparseable_is_none() {
        let got = parse_judgment("2026-08-24", "sure, buy everything", &[cand("SHOP")], "x");
        assert!(got.ticker.is_none());
        assert!(got.why.contains("No pick invented"));
    }
}
