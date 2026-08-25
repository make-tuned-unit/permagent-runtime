//! The baselines every other method has to beat — pure, DB-free, no network.
//!
//! Two methods live here and nothing else does:
//!
//! * **Seasonal naive.** Tomorrow looks like the same weekday last week. It is
//!   the denominator of MASE and the thing a foundation model has to beat to
//!   earn its label. On daily equity closes it is very often the honest answer,
//!   which the spike's NVDA result (MASE 1.104 against it) demonstrated.
//! * **Holt-Winters additive ETS.** Level, damped trend, and an additive
//!   seasonal term, with the smoothing parameters chosen by grid search over
//!   in-sample squared error. Deterministic: the same series always yields the
//!   same fit, so a test can pin it.
//!
//! Both are DB-free by design, the way `growth::power` is — the arithmetic that
//! decides what may be claimed must be testable without a database, or it will
//! not be tested at scale.

/// Seasonal-naive: repeat the last full season.
///
/// `m = 1` degenerates to naive-last, which is the right baseline for a series
/// with no registered seasonality (an equity close, for one).
///
/// Returns an empty vector when there is nothing to repeat — a caller must
/// treat that as "no forecast", never as zeros.
pub fn seasonal_naive(y: &[f64], h: usize, m: usize) -> Vec<f64> {
    if y.is_empty() || h == 0 {
        return Vec::new();
    }
    let m = m.max(1);
    if y.len() < m {
        // Not one full season yet: the last observation is all we have.
        return vec![y[y.len() - 1]; h];
    }
    let base = y.len() - m;
    (0..h).map(|i| y[base + (i % m)]).collect()
}

/// A fitted Holt-Winters state, kept so the fit and the forecast cannot drift
/// apart.
#[derive(Debug, Clone, PartialEq)]
pub struct EtsFit {
    pub alpha: f64,
    pub beta: f64,
    pub gamma: f64,
    pub level: f64,
    pub trend: f64,
    pub season: Vec<f64>,
    /// In-sample one-step squared error. Lower is a better fit, not a better
    /// forecast — that is what the backtest is for.
    pub sse: f64,
    /// One-step in-sample residuals, used to widen the interval honestly rather
    /// than from an assumed distribution.
    pub residuals: Vec<f64>,
}

/// Damping on the trend. An undamped Holt-Winters line extrapolated a week out
/// is usually a straight-line fantasy; 0.95 is the conventional gentle damp and
/// keeps the forecast from running away at longer horizons.
const PHI: f64 = 0.95;

const GRID: [f64; 5] = [0.05, 0.2, 0.4, 0.6, 0.85];

/// Fit additive Holt-Winters by grid search over (alpha, beta, gamma).
///
/// Needs two full seasons: one to initialise the seasonal terms and one to have
/// anything to fit against. Returns `None` below that rather than fitting
/// something meaningless.
pub fn fit_ets(y: &[f64], m: usize) -> Option<EtsFit> {
    let m = m.max(1);
    if y.len() < 2 * m || y.len() < 4 {
        return None;
    }
    let mut best: Option<EtsFit> = None;
    for &alpha in &GRID {
        for &beta in &GRID {
            for &gamma in &GRID {
                if let Some(fit) = fit_once(y, m, alpha, beta, gamma) {
                    if best.as_ref().is_none_or(|b| fit.sse < b.sse) {
                        best = Some(fit);
                    }
                }
            }
        }
    }
    best
}

fn fit_once(y: &[f64], m: usize, alpha: f64, beta: f64, gamma: f64) -> Option<EtsFit> {
    let n = y.len();
    let first: f64 = y[..m].iter().sum::<f64>() / m as f64;
    let second: f64 = y[m..2 * m].iter().sum::<f64>() / m as f64;
    let mut level = first;
    let mut trend = (second - first) / m as f64;
    let mut season: Vec<f64> = y[..m].iter().map(|v| v - first).collect();
    let mut sse = 0.0;
    let mut residuals = Vec::with_capacity(n.saturating_sub(m));

    for (t, &yt) in y.iter().enumerate().skip(m) {
        let s_idx = t % m;
        let fitted = level + PHI * trend + season[s_idx];
        let err = yt - fitted;
        if !err.is_finite() {
            return None;
        }
        sse += err * err;
        residuals.push(err);

        let prev_level = level;
        level = alpha * (yt - season[s_idx]) + (1.0 - alpha) * (level + PHI * trend);
        trend = beta * (level - prev_level) + (1.0 - beta) * PHI * trend;
        season[s_idx] = gamma * (yt - level) + (1.0 - gamma) * season[s_idx];
        if !level.is_finite() || !trend.is_finite() {
            return None;
        }
    }
    sse.is_finite().then_some(EtsFit {
        alpha,
        beta,
        gamma,
        level,
        trend,
        season,
        sse,
        residuals,
    })
}

/// Project a fitted state forward.
pub fn ets_forecast(fit: &EtsFit, y_len: usize, h: usize) -> Vec<f64> {
    let m = fit.season.len().max(1);
    let mut damp = 0.0;
    (0..h)
        .map(|i| {
            damp += PHI.powi(i as i32 + 1);
            fit.level + damp * fit.trend + fit.season[(y_len + i) % m]
        })
        .collect()
}

/// Fit and project in one call. `None` when the series is too short to fit.
pub fn ets(y: &[f64], h: usize, m: usize) -> Option<Vec<f64>> {
    let fit = fit_ets(y, m)?;
    Some(ets_forecast(&fit, y.len(), h))
}

/// Mean absolute scaled error (Hyndman): mean absolute error over the holdout,
/// divided by the in-sample mean absolute seasonal-naive error.
///
/// `None` when the denominator is zero — a perfectly flat training window makes
/// MASE undefined, and an undefined number must not be reported as a good one.
pub fn mase(actual: &[f64], forecast: &[f64], train: &[f64], m: usize) -> Option<f64> {
    let m = m.max(1);
    if actual.is_empty() || actual.len() != forecast.len() || train.len() <= m {
        return None;
    }
    let denom: f64 = train
        .windows(m + 1)
        .map(|w| (w[m] - w[0]).abs())
        .sum::<f64>()
        / (train.len() - m) as f64;
    if !denom.is_finite() || denom <= f64::EPSILON {
        return None;
    }
    let numer: f64 = actual
        .iter()
        .zip(forecast)
        .map(|(a, f)| (a - f).abs())
        .sum::<f64>()
        / actual.len() as f64;
    numer.is_finite().then(|| numer / denom)
}

/// An 80% interval built from the method's own one-step residuals, widened as
/// sqrt(h) the way an accumulating random walk does.
///
/// Deliberately empirical: a Gaussian interval on a download count would be
/// symmetric and often negative, and a negative download count is a tell that
/// the interval was assumed rather than measured.
pub fn residual_interval(
    point: &[f64],
    residuals: &[f64],
    non_negative: bool,
) -> (Vec<f64>, Vec<f64>) {
    if residuals.len() < 8 {
        // Not enough residuals to quantify anything. An interval equal to the
        // point estimate is visibly useless, which is the honest rendering.
        return (point.to_vec(), point.to_vec());
    }
    let mut sorted: Vec<f64> = residuals
        .iter()
        .copied()
        .filter(|r| r.is_finite())
        .collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |p: f64| -> f64 {
        let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    };
    let (lo_r, hi_r) = (q(0.10), q(0.90));
    let mut lo = Vec::with_capacity(point.len());
    let mut hi = Vec::with_capacity(point.len());
    for (i, p) in point.iter().enumerate() {
        let widen = ((i + 1) as f64).sqrt();
        let mut l = p + lo_r * widen;
        let h = p + hi_r * widen;
        if non_negative && l < 0.0 {
            l = 0.0;
        }
        lo.push(l);
        hi.push(h);
    }
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seasonal_naive_reproduces_a_hand_computed_forecast() {
        let y: Vec<f64> = (1..=14).map(f64::from).collect();
        // n = 14, m = 7: the last full week is y[7..14] = 8..14, repeated.
        assert_eq!(seasonal_naive(&y, 3, 7), vec![8.0, 9.0, 10.0]);
        // Wrapping past one season repeats it.
        assert_eq!(
            seasonal_naive(&y, 9, 7),
            vec![8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 8.0, 9.0]
        );
        // m = 1 is naive-last, the right baseline for an unseasonal series.
        assert_eq!(seasonal_naive(&y, 3, 1), vec![14.0, 14.0, 14.0]);
        // Shorter than one season: the last observation, not a wrong wrap.
        assert_eq!(seasonal_naive(&[5.0, 6.0], 2, 7), vec![6.0, 6.0]);
        assert!(seasonal_naive(&[], 3, 7).is_empty());
    }

    #[test]
    fn ets_tracks_a_clean_trend_plus_season() {
        // level 100, +0.5/day, weekly amplitude 6.
        let y: Vec<f64> = (0..84)
            .map(|t| {
                let t = t as f64;
                100.0 + 0.5 * t + 6.0 * (2.0 * std::f64::consts::PI * t / 7.0).sin()
            })
            .collect();
        let f = ets(&y, 7, 7).expect("84 points is twelve seasons");
        // Every step must be within a few percent of the truth it was built from.
        for (i, v) in f.iter().enumerate() {
            let t = (84 + i) as f64;
            let truth = 100.0 + 0.5 * t + 6.0 * (2.0 * std::f64::consts::PI * t / 7.0).sin();
            assert!(
                (v - truth).abs() < 0.10 * truth.abs().max(1.0),
                "step {i}: fitted {v:.2} vs truth {truth:.2}"
            );
        }
        // And it beats seasonal-naive on this series, which is the whole point
        // of carrying a second baseline.
        let train = &y[..77];
        let actual = &y[77..];
        let ets_mase = mase(actual, &ets(train, 7, 7).unwrap(), train, 7).unwrap();
        let sn_mase = mase(actual, &seasonal_naive(train, 7, 7), train, 7).unwrap();
        assert!(
            ets_mase < sn_mase,
            "ets {ets_mase:.3} vs naive {sn_mase:.3}"
        );
    }

    #[test]
    fn ets_refuses_a_series_shorter_than_two_seasons() {
        let y: Vec<f64> = (0..13).map(f64::from).collect();
        assert!(fit_ets(&y, 7).is_none());
        assert!(ets(&y, 3, 7).is_none());
    }

    /// A PERFECTLY seasonal series makes MASE undefined, not zero: the
    /// denominator it is scaled by IS the in-sample seasonal-naive error, which
    /// is exactly 0 there. That is the trap this test exists to pin — an
    /// earlier draft of it called `.unwrap()` and panicked.
    #[test]
    fn a_perfectly_seasonal_series_makes_mase_undefined_not_zero() {
        let y: Vec<f64> = (0..28)
            .map(|t| [1.0, 5.0, 3.0, 9.0, 2.0, 7.0, 4.0][t % 7])
            .collect();
        let train = &y[..21];
        let actual = &y[21..];
        assert_eq!(
            mase(actual, &seasonal_naive(train, 7, 7), train, 7),
            None,
            "a zero denominator is undefined, never a perfect score"
        );
    }

    /// Seasonal naive scores about 1.0 against its own denominator — that is
    /// what makes MASE readable at a glance, and it is not an accident: the
    /// denominator IS the in-sample seasonal-naive error, so out-of-sample it
    /// lands near 1 whenever the noise is stationary. A method is worth its
    /// name only by scoring meaningfully below that, which is what the gate's
    /// 10% margin is measured against.
    #[test]
    fn seasonal_naive_scores_about_one_against_its_own_denominator() {
        let base = [1.0, 5.0, 3.0, 9.0, 2.0, 7.0, 4.0];
        let y: Vec<f64> = (0..56)
            .map(|t| base[t % 7] * 10.0 + ((t * 37) % 11) as f64 * 0.1)
            .collect();
        let train = &y[..49];
        let actual = &y[49..];
        let m = mase(actual, &seasonal_naive(train, 7, 7), train, 7)
            .expect("a noisy series has a real denominator");
        assert!(m.is_finite(), "got {m}");
        assert!(
            (0.3..=3.0).contains(&m),
            "seasonal naive should land near 1, got {m}"
        );
    }

    /// And ETS, given a trend seasonal naive cannot see, does score below it.
    #[test]
    fn ets_beats_seasonal_naive_when_there_is_a_trend_to_see() {
        let y: Vec<f64> = (0..120)
            .map(|t| {
                let t = t as f64;
                200.0 + 1.5 * t + 8.0 * (2.0 * std::f64::consts::PI * t / 7.0).sin()
            })
            .collect();
        let train = &y[..113];
        let actual = &y[113..];
        let sn = mase(actual, &seasonal_naive(train, 7, 7), train, 7).unwrap();
        let e = mase(actual, &ets(train, 7, 7).unwrap(), train, 7).unwrap();
        assert!(e < sn, "ets {e:.3} must beat seasonal naive {sn:.3}");
    }

    #[test]
    fn mase_is_undefined_rather_than_flattering_on_a_flat_window() {
        let flat = vec![4.0; 30];
        assert!(
            mase(&[4.0, 4.0], &[4.0, 4.0], &flat, 7).is_none(),
            "a zero denominator is undefined, not a perfect score"
        );
    }

    #[test]
    fn an_interval_on_a_count_series_never_goes_negative() {
        let point = vec![5.0, 5.0, 5.0];
        let residuals: Vec<f64> = (0..40).map(|i| (i as f64 % 11.0) - 5.5).collect();
        let (lo, hi) = residual_interval(&point, &residuals, true);
        assert!(lo.iter().all(|v| *v >= 0.0), "{lo:?}");
        assert!(hi.iter().zip(&lo).all(|(h, l)| h >= l));
        // And it widens with the horizon rather than staying flat.
        assert!(hi[2] > hi[0]);
    }

    #[test]
    fn too_few_residuals_yields_no_interval_rather_than_a_made_up_one() {
        let point = vec![5.0, 5.0];
        let (lo, hi) = residual_interval(&point, &[1.0, -1.0], true);
        assert_eq!(lo, point);
        assert_eq!(hi, point);
    }
}
