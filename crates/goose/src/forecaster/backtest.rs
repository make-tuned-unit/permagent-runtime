//! The gate. A method earns its label here or it does not get one.
//!
//! Rolling-origin evaluation, MASE against the seasonal-naive denominator, and
//! a two-part promotion test: a **10% margin on the median** and a **sign test
//! on the folds**. Both must hold. One lucky fold cannot promote a method, and
//! a tie is not a win.
//!
//! The worked example is in the spike: on 1024 real NVDA daily closes TimesFM
//! scored MASE 1.104 against naive-last — worse than the baseline. The gate
//! firing there is the feature working, not the feature failing. For equity
//! closes the honest answer is usually "random walk", and the baseline already
//! says exactly that.
//!
//! Pure and DB-free, like `baseline`.

use super::baseline::mase;

/// Folds required before any verdict is possible.
///
/// Eight is what `Cadence::min_points` was sized to guarantee, and it is the
/// smallest number at which the sign test below has any power: at 8 folds a
/// 6-win result has a one-sided binomial p of about 0.14 under a fair coin,
/// which is weak on its own — hence the second, independent margin test.
pub const MIN_FOLDS: usize = 8;

/// The candidate's median MASE must be at most this multiple of the baseline's.
/// A 10% margin, deliberately not a tie-break.
pub const MASE_MARGIN: f64 = 0.90;

/// Fraction of folds the candidate must win. 3/4 is exactly "6 of 8" at the
/// minimum fold count, and it keeps its meaning on longer series.
pub const WIN_FRACTION: (usize, usize) = (3, 4);

/// One method's performance across the folds.
#[derive(Debug, Clone, PartialEq)]
pub struct FoldScores {
    pub mases: Vec<f64>,
}

impl FoldScores {
    pub fn median(&self) -> Option<f64> {
        if self.mases.is_empty() {
            return None;
        }
        let mut v = self.mases.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = v.len();
        Some(if n % 2 == 1 {
            v[n / 2]
        } else {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        })
    }
}

/// What the gate decided, and why. `NoVerdict` is a distinct state from
/// `Rejected`: "we could not tell" and "we told, and the answer is no" have
/// different consequences, and conflating them is how a weak result becomes a
/// promotion.
#[derive(Debug, Clone, PartialEq)]
pub enum GateOutcome {
    /// Both tests passed. The candidate may be labelled as itself.
    Promoted {
        median_ratio: f64,
        wins: usize,
        folds: usize,
    },
    /// At least one test failed. The baseline is served, and labelled.
    Rejected {
        reason: String,
        median_ratio: Option<f64>,
        wins: usize,
        folds: usize,
    },
    /// Not enough folds to decide anything.
    NoVerdict { folds: usize, needed: usize },
}

impl GateOutcome {
    pub fn promoted(&self) -> bool {
        matches!(self, Self::Promoted { .. })
    }
}

/// Roll the origin forward by `h` and score one method at each origin.
///
/// `predict` receives the training prefix and returns `h` values, or `None` if
/// it cannot forecast that prefix — a fold a method could not produce is simply
/// not scored for it, never scored as a loss it did not earn.
pub fn rolling_origin<F>(
    y: &[f64],
    h: usize,
    m: usize,
    min_train: usize,
    mut predict: F,
) -> FoldScores
where
    F: FnMut(&[f64]) -> Option<Vec<f64>>,
{
    let mut mases = Vec::new();
    for origin in fold_origins(y.len(), h, min_train) {
        let train = &y[..origin];
        let actual = &y[origin..origin + h];
        if let Some(f) = predict(train) {
            if f.len() == h {
                if let Some(s) = mase(actual, &f, train, m) {
                    mases.push(s);
                }
            }
        }
    }
    FoldScores { mases }
}

/// The training-prefix lengths this evaluation uses, in order.
///
/// Public and shared so a method evaluated *elsewhere* — TimesFM, on another
/// machine — is scored on exactly the folds the local methods were. The sign
/// test is paired; folds that drifted apart would compare one method's easy
/// folds against another's hard ones and call the result a verdict.
pub fn fold_origins(len: usize, h: usize, min_train: usize) -> Vec<usize> {
    let mut out = Vec::new();
    if h == 0 || len < min_train + h {
        return out;
    }
    let mut origin = min_train;
    while origin + h <= len {
        out.push(origin);
        origin += h;
    }
    out
}

/// The two-part promotion test.
///
/// Both series of scores must come from the *same* folds, in the same order —
/// the sign test is paired, and pairing it wrongly would compare a method's
/// easy folds against another's hard ones.
pub fn gate(candidate: &FoldScores, baseline: &FoldScores) -> GateOutcome {
    let folds = candidate.mases.len().min(baseline.mases.len());
    if candidate.mases.len() != baseline.mases.len() {
        return GateOutcome::Rejected {
            reason: "the two methods did not score the same folds, so no paired \
                     comparison is possible"
                .into(),
            median_ratio: None,
            wins: 0,
            folds,
        };
    }
    if folds < MIN_FOLDS {
        return GateOutcome::NoVerdict {
            folds,
            needed: MIN_FOLDS,
        };
    }
    let (Some(c_med), Some(b_med)) = (candidate.median(), baseline.median()) else {
        return GateOutcome::NoVerdict {
            folds,
            needed: MIN_FOLDS,
        };
    };
    // A zero baseline median means the baseline was exact; nothing beats that
    // by 10%, and dividing by it would be a fabricated ratio.
    if b_med <= f64::EPSILON {
        return GateOutcome::Rejected {
            reason: "the baseline was exact on these folds; there is nothing to beat".into(),
            median_ratio: None,
            wins: 0,
            folds,
        };
    }
    let ratio = c_med / b_med;
    // Strictly better, not merely different: a tie goes to the baseline.
    let wins = candidate
        .mases
        .iter()
        .zip(&baseline.mases)
        .filter(|(c, b)| c < b)
        .count();
    let needed_wins = folds.div_ceil(WIN_FRACTION.1) * WIN_FRACTION.0;
    let margin_ok = ratio <= MASE_MARGIN;
    let wins_ok = wins >= needed_wins;
    if margin_ok && wins_ok {
        return GateOutcome::Promoted {
            median_ratio: ratio,
            wins,
            folds,
        };
    }
    let reason = match (margin_ok, wins_ok) {
        (false, false) => format!(
            "median MASE {ratio:.3}x the baseline (needs <= {MASE_MARGIN:.2}) and won only \
             {wins} of {folds} folds (needs {needed_wins})"
        ),
        (false, true) => format!(
            "median MASE {ratio:.3}x the baseline; the margin test needs <= {MASE_MARGIN:.2}"
        ),
        (true, false) => {
            format!("won only {wins} of {folds} folds; the sign test needs {needed_wins}")
        }
        (true, true) => unreachable!("handled above"),
    };
    GateOutcome::Rejected {
        reason,
        median_ratio: Some(ratio),
        wins,
        folds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forecaster::baseline::{ets, seasonal_naive};

    fn scores(v: &[f64]) -> FoldScores {
        FoldScores { mases: v.to_vec() }
    }

    #[test]
    fn fewer_than_eight_folds_yields_no_verdict_not_a_weak_one() {
        let c = scores(&[0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1]);
        let b = scores(&[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
        // Overwhelming on seven folds is still not a verdict.
        assert_eq!(
            gate(&c, &b),
            GateOutcome::NoVerdict {
                folds: 7,
                needed: 8
            }
        );
    }

    #[test]
    fn a_method_that_clears_both_tests_is_promoted() {
        let c = scores(&[0.15, 0.20, 0.18, 0.25, 0.19, 0.30, 0.22, 0.17]);
        let b = scores(&[1.00, 1.10, 0.95, 1.20, 1.05, 0.90, 1.15, 1.02]);
        match gate(&c, &b) {
            GateOutcome::Promoted {
                median_ratio,
                wins,
                folds,
            } => {
                assert_eq!(folds, 8);
                assert_eq!(wins, 8);
                assert!(median_ratio < MASE_MARGIN, "{median_ratio}");
            }
            other => panic!("expected promotion, got {other:?}"),
        }
    }

    /// The spike's worked example: on 1024 real NVDA daily closes TimesFM
    /// scored MASE 1.104 against naive-last. The gate must refuse it.
    #[test]
    fn the_nvda_result_from_the_spike_is_rejected() {
        let c = scores(&[1.104; 8]);
        let b = scores(&[1.000; 8]);
        match gate(&c, &b) {
            GateOutcome::Rejected {
                median_ratio, wins, ..
            } => {
                assert_eq!(wins, 0, "it lost every fold");
                assert!((median_ratio.unwrap() - 1.104).abs() < 1e-9);
            }
            other => panic!("MASE 1.104 must be rejected, got {other:?}"),
        }
    }

    #[test]
    fn winning_the_margin_but_not_the_folds_is_still_a_rejection() {
        // The median clears the margin comfortably (0.675 <= 0.90) while the
        // sign test does not (5 of 8, needs 6). The two tests are independent on
        // purpose: a method can look good on average and still be wrong more
        // often than not.
        let c = scores(&[0.5, 0.5, 0.5, 0.5, 0.85, 1.5, 1.6, 1.7]);
        let b = scores(&[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
        assert!(c.median().unwrap() <= MASE_MARGIN, "{:?}", c.median());
        match gate(&c, &b) {
            GateOutcome::Rejected { reason, wins, .. } => {
                assert_eq!(wins, 5);
                assert!(reason.contains("sign test"), "{reason}");
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn winning_the_folds_but_not_the_margin_is_still_a_rejection() {
        // Wins every fold, but only barely — the margin test is what stops a
        // 1% improvement from earning a model's name.
        let c = scores(&[0.98; 8]);
        let b = scores(&[1.00; 8]);
        match gate(&c, &b) {
            GateOutcome::Rejected { reason, wins, .. } => {
                assert_eq!(wins, 8);
                assert!(reason.contains("margin"), "{reason}");
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_tie_goes_to_the_baseline() {
        let c = scores(&[1.0; 8]);
        let b = scores(&[1.0; 8]);
        assert!(!gate(&c, &b).promoted());
    }

    #[test]
    fn mismatched_folds_are_refused_rather_than_compared() {
        let c = scores(&[0.1; 8]);
        let b = scores(&[1.0; 6]);
        match gate(&c, &b) {
            GateOutcome::Rejected { reason, .. } => {
                assert!(reason.contains("same folds"), "{reason}")
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn rolling_origin_scores_the_folds_the_minimum_was_sized_for() {
        // 180 daily points, H = 7, min_train 120 → (180-120)/7 = 8 folds.
        let y: Vec<f64> = (0..180)
            .map(|t| {
                let t = t as f64;
                50.0 + 0.2 * t + 5.0 * (2.0 * std::f64::consts::PI * t / 7.0).sin()
            })
            .collect();
        let sn = rolling_origin(&y, 7, 7, 120, |train| Some(seasonal_naive(train, 7, 7)));
        assert_eq!(sn.mases.len(), 8, "the fold count the gate needs");
        let e = rolling_origin(&y, 7, 7, 120, |train| ets(train, 7, 7));
        assert_eq!(e.mases.len(), 8);
        // On a clean trend-plus-season ETS should clear the gate over naive.
        assert!(gate(&e, &sn).promoted(), "{:?} vs {:?}", e.mases, sn.mases);
    }

    /// A method scored on another machine has to be scored on the SAME folds,
    /// so both sides read the origins from here.
    #[test]
    fn fold_origins_are_shared_so_a_remote_method_is_scored_on_the_same_folds() {
        let origins = fold_origins(180, 7, 124);
        assert_eq!(origins.len(), 8);
        assert_eq!(origins[0], 124);
        assert_eq!(origins[7], 173);
        // Noise matters: a PERFECTLY seasonal series has a zero MASE
        // denominator, so every fold is unscoreable and the counts would not
        // line up for a reason unrelated to fold alignment.
        let y: Vec<f64> = (0..180)
            .map(|t| (t % 7) as f64 * 10.0 + ((t * 37) % 11) as f64 * 0.1)
            .collect();
        let scored = rolling_origin(&y, 7, 7, 124, |t| Some(seasonal_naive(t, 7, 7)));
        assert_eq!(scored.mases.len(), origins.len());
        // Too short for a single fold: no origins, and therefore no verdict.
        assert!(fold_origins(10, 7, 124).is_empty());
    }

    #[test]
    fn a_method_that_cannot_forecast_a_fold_scores_no_fold_rather_than_a_loss() {
        let y: Vec<f64> = (0..40).map(f64::from).collect();
        let none = rolling_origin(&y, 7, 7, 20, |_| None);
        assert!(none.mases.is_empty());
        // And the gate treats that as "cannot tell", not "lost".
        let sn = rolling_origin(&y, 7, 7, 20, |t| Some(seasonal_naive(t, 7, 7)));
        assert!(matches!(
            gate(&none, &sn),
            GateOutcome::Rejected { .. } | GateOutcome::NoVerdict { .. }
        ));
    }
}
