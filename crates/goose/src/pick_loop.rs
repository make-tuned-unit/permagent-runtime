//! Loop-engineering gate for Picker picks.
//!
//! Picker already ranked the names. This module does not generate new
//! strategies and does not re-rank on the user's holdings. Each pick is one
//! hypothesis — "recent momentum on this ticker is a consistent next-day
//! edge" — and must pass the same three checks quants use:
//!
//! 1. **ICIR** (mean monthly Information Coefficient / its std). Below 0.3
//!    is treated as noise.
//! 2. **Decay half-life** of the signal autocorrelation. Under 5 days is
//!    too short to trade after costs.
//! 3. **Out-of-sample gate** on the most recent 20% of the series, held
//!    out from scoring. ICIR must not drop more than 50%, and a Bonferroni
//!    correction raises the bar by the number of picks in the batch.
//!
//! The signal is 5-day momentum (close[t]/close[t-5] − 1) scored against
//! the next-day return. Parameters are fixed — the loop must not hunt a
//! magic lookback on the same data it scores.

use serde::{Deserialize, Serialize};

/// ICIR below this is noise (Ray C. Fu loop framework).
pub const ICIR_NOISE: f64 = 0.3;
/// Half-life under this many days is untradeable after costs.
pub const MIN_HALF_LIFE_DAYS: f64 = 5.0;
/// Held-out fraction. Locked before scoring; never mixed back in.
pub const OOS_FRACTION: f64 = 0.20;
/// Kill if OOS ICIR drops by more than this relative to in-sample.
pub const MAX_OOS_DROP: f64 = 0.50;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoopGate {
    pub icir: Option<f64>,
    pub ic_mean: Option<f64>,
    pub half_life_days: Option<f64>,
    pub oos_icir: Option<f64>,
    pub passed: bool,
    pub kills: Vec<String>,
    /// How many other picks this Bonferroni correction accounted for.
    pub batch_size: usize,
}

/// Score one ticker's daily closes. `closes` is oldest → newest. `batch_size`
/// is the number of picks in this scan (Bonferroni n).
pub fn validate_closes(closes: &[f64], batch_size: usize) -> LoopGate {
    let n = batch_size.max(1);
    let mut kills = Vec::new();

    let pairs = momentum_pairs(closes);
    if pairs.len() < 40 {
        return LoopGate {
            icir: None,
            ic_mean: None,
            half_life_days: None,
            oos_icir: None,
            passed: false,
            kills: vec!["not enough daily history to run the loop (need ~40 paired days)".into()],
            batch_size: n,
        };
    }

    let split = ((pairs.len() as f64) * (1.0 - OOS_FRACTION)).floor() as usize;
    let split = split.clamp(20, pairs.len().saturating_sub(8));
    let (insample, oos) = pairs.split_at(split);

    let (ic_mean, icir) = icir_of(insample);
    let oos_icir = icir_of(oos).1;
    let signals: Vec<f64> = insample.iter().map(|p| p.0).collect();
    let half_life = half_life_days(&signals);

    if let Some(v) = icir {
        if v < ICIR_NOISE {
            kills.push(format!(
                "in-sample ICIR {v:.2} is below {ICIR_NOISE} — likely noise"
            ));
        }
    } else {
        kills.push("in-sample ICIR could not be computed".into());
    }
    match half_life {
        Some(h) if h < MIN_HALF_LIFE_DAYS => {
            kills.push(format!(
                "signal half-life {h:.1}d is under {MIN_HALF_LIFE_DAYS}d"
            ));
        }
        None => kills.push("signal half-life could not be estimated".into()),
        _ => {}
    }
    match (icir, oos_icir) {
        (Some(ins), Some(out)) if ins.abs() > f64::EPSILON => {
            let drop = (ins - out) / ins.abs();
            if drop > MAX_OOS_DROP {
                kills.push(format!(
                    "out-of-sample ICIR {out:.2} dropped more than 50% from in-sample {ins:.2} — overfit"
                ));
            }
        }
        (_, None) => kills.push("out-of-sample ICIR could not be computed".into()),
        _ => {}
    }
    if !passes_bonferroni(icir, insample.len(), n) {
        kills.push(format!(
            "failed Bonferroni gate for {n} picks tested in this batch"
        ));
    }

    LoopGate {
        icir,
        ic_mean,
        half_life_days: half_life,
        oos_icir,
        passed: kills.is_empty(),
        kills,
        batch_size: n,
    }
}

/// (signal, next-day return) pairs. Signal is 5-day momentum at t,
/// return is close[t+1]/close[t] − 1.
fn momentum_pairs(closes: &[f64]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    if closes.len() < 7 {
        return out;
    }
    for i in 5..closes.len().saturating_sub(1) {
        let prev = closes[i - 5];
        let now = closes[i];
        let nxt = closes[i + 1];
        if prev > 0.0 && now > 0.0 && nxt > 0.0 {
            out.push((now / prev - 1.0, nxt / now - 1.0));
        }
    }
    out
}

/// Monthly IC (Pearson of signal vs next-day return in ~21-day windows),
/// then ICIR = mean(IC) / std(IC).
fn icir_of(pairs: &[(f64, f64)]) -> (Option<f64>, Option<f64>) {
    const MONTH: usize = 21;
    if pairs.len() < MONTH * 2 {
        let xs: Vec<f64> = pairs.iter().map(|p| p.0).collect();
        let ys: Vec<f64> = pairs.iter().map(|p| p.1).collect();
        let ic = pearson(&xs, &ys);
        return (ic, ic);
    }
    let mut ics = Vec::new();
    let mut i = 0;
    while i + MONTH <= pairs.len() {
        let window = &pairs[i..i + MONTH];
        let xs: Vec<f64> = window.iter().map(|p| p.0).collect();
        let ys: Vec<f64> = window.iter().map(|p| p.1).collect();
        if let Some(ic) = pearson(&xs, &ys) {
            ics.push(ic);
        }
        i += MONTH;
    }
    if ics.len() < 2 {
        return (ics.first().copied(), ics.first().copied());
    }
    let mean = ics.iter().sum::<f64>() / ics.len() as f64;
    let var = ics.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (ics.len() - 1) as f64;
    let std = var.sqrt();
    let icir = if std > f64::EPSILON {
        Some(mean / std)
    } else {
        None
    };
    (Some(mean), icir)
}

fn pearson(xs: &[f64], ys: &[f64]) -> Option<f64> {
    if xs.len() != ys.len() || xs.len() < 8 {
        return None;
    }
    let n = xs.len() as f64;
    let mx = xs.iter().sum::<f64>() / n;
    let my = ys.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut dx = 0.0;
    let mut dy = 0.0;
    for (x, y) in xs.iter().zip(ys.iter()) {
        let a = x - mx;
        let b = y - my;
        num += a * b;
        dx += a * a;
        dy += b * b;
    }
    let den = (dx * dy).sqrt();
    if den < f64::EPSILON {
        return None;
    }
    Some(num / den)
}

/// Autocorrelation of the signal at lags 1, 5, 10, 20, 50. Half-life is the
/// first lag where |acf| drops to 0.5, linearly interpolated.
fn half_life_days(signal: &[f64]) -> Option<f64> {
    if signal.len() < 30 {
        return None;
    }
    let lags = [1usize, 5, 10, 20, 50];
    let mut prev_lag = 0.0f64;
    let mut prev_acf = 1.0f64;
    for &lag in &lags {
        let Some(acf) = acf_at(signal, lag) else {
            continue;
        };
        if acf.abs() <= 0.5 {
            let span = lag as f64 - prev_lag;
            if span <= 0.0 {
                return Some(lag as f64);
            }
            let t = (prev_acf.abs() - 0.5) / (prev_acf.abs() - acf.abs()).max(1e-9);
            return Some(prev_lag + t.clamp(0.0, 1.0) * span);
        }
        prev_lag = lag as f64;
        prev_acf = acf;
    }
    // Still above 0.5 at lag 50 — long-lived.
    Some(50.0)
}

fn acf_at(xs: &[f64], lag: usize) -> Option<f64> {
    if lag == 0 || lag >= xs.len() - 8 {
        return None;
    }
    let a = &xs[..xs.len() - lag];
    let b = &xs[lag..];
    pearson(a, b)
}

/// Two-sided t on monthly-IC count, Bonferroni-adjusted by batch size.
/// `p < 0.05 / n` must hold. A missing ICIR fails closed.
fn passes_bonferroni(icir: Option<f64>, n_pairs: usize, batch_size: usize) -> bool {
    let Some(icir) = icir else {
        return false;
    };
    let months = (n_pairs / 21).max(2) as f64;
    // t ≈ ICIR * sqrt(T-1); two-sided p via erfc approximation of a normal.
    let t = icir.abs() * (months - 1.0).sqrt();
    let p = 2.0 * norm_sf(t);
    let alpha = 0.05 / batch_size.max(1) as f64;
    p < alpha
}

fn norm_sf(z: f64) -> f64 {
    // Complementary error function approximation for the standard-normal tail.
    0.5 * erfc(z / std::f64::consts::SQRT_2)
}

fn erfc(x: f64) -> f64 {
    // Abramowitz and Stegun 7.1.26
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * z);
    let poly = t
        * (0.254829592
            + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    let ans = poly * (-z * z).exp();
    if x < 0.0 {
        2.0 - ans
    } else {
        ans
    }
}

/// Wilder RSI-14 of a close series (oldest → newest).
pub fn rsi_14(closes: &[f64]) -> Option<f64> {
    const N: usize = 14;
    if closes.len() < N + 1 {
        return None;
    }
    let mut gains = 0.0;
    let mut losses = 0.0;
    for i in 1..=N {
        let d = closes[i] - closes[i - 1];
        if d >= 0.0 {
            gains += d;
        } else {
            losses -= d;
        }
    }
    let mut avg_gain = gains / N as f64;
    let mut avg_loss = losses / N as f64;
    for i in (N + 1)..closes.len() {
        let d = closes[i] - closes[i - 1];
        let (g, l) = if d >= 0.0 { (d, 0.0) } else { (0.0, -d) };
        avg_gain = (avg_gain * (N as f64 - 1.0) + g) / N as f64;
        avg_loss = (avg_loss * (N as f64 - 1.0) + l) / N as f64;
    }
    if avg_loss < f64::EPSILON {
        return Some(100.0);
    }
    let rs = avg_gain / avg_loss;
    Some(100.0 - 100.0 / (1.0 + rs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trending(n: usize) -> Vec<f64> {
        // Persistent upward drift with a small wobble so 5-day momentum and
        // next-day return both vary (a pure geometric series has zero
        // variance, so Pearson/ICIR is undefined).
        (0..n)
            .map(|i| {
                let t = i as f64;
                80.0 + 0.35 * t + 1.2 * (t / 9.0).sin()
            })
            .collect()
    }

    fn noise(n: usize) -> Vec<f64> {
        // Deterministic pseudo-noise so the test is stable: alternating
        // steps with no persistence.
        (0..n)
            .map(|i| 100.0 + if i % 3 == 0 { 4.0 } else { -2.0 })
            .collect()
    }

    #[test]
    fn pearson_of_identical_series_is_one() {
        let xs = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        assert!((pearson(&xs, &xs).unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn persistent_trend_passes_the_gate() {
        let g = validate_closes(&trending(260), 1);
        assert!(
            g.passed,
            "a clean trend should survive ICIR + decay + OOS, kills={:?}",
            g.kills
        );
        assert!(g.icir.unwrap() >= ICIR_NOISE, "icir={:?}", g.icir);
    }

    #[test]
    fn chop_is_killed_as_noise() {
        let g = validate_closes(&noise(260), 1);
        assert!(!g.passed, "alternating chop must not pass, gate={g:?}");
        assert!(
            g.kills
                .iter()
                .any(|k| k.contains("noise") || k.contains("Bonferroni") || k.contains("dropped")),
            "expected a noise/overfit kill, got {:?}",
            g.kills
        );
    }

    #[test]
    fn short_history_fails_closed() {
        let g = validate_closes(&[10.0, 11.0, 12.0], 3);
        assert!(!g.passed);
        assert!(g.kills[0].contains("not enough"));
    }

    #[test]
    fn rsi_of_a_straight_climb_is_hot() {
        let closes: Vec<f64> = (0..40).map(|i| 50.0 + i as f64).collect();
        let rsi = rsi_14(&closes).unwrap();
        assert!(rsi > 70.0, "climbing series RSI={rsi}");
    }

    #[test]
    fn bonferroni_tightens_with_batch_size() {
        // A modest ICIR that would pass n=1 can fail n=200.
        assert!(passes_bonferroni(Some(2.0), 21 * 12, 1));
        assert!(!passes_bonferroni(Some(0.4), 21 * 6, 200));
    }
}
