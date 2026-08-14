//! The power check and the verdict.
//!
//! This is the file the proposal's warning is aimed at:
//!
//! > A feature that says "this helped, +12%" off 40 pageviews is not measuring;
//! > it is pattern-matching noise and presenting it as evidence. It then does
//! > something worse: step 5 feeds that noise back into future strategy.
//!
//! So no verdict is produced without first asking whether a delta that size
//! could have come from this project's ordinary week-to-week movement. When it
//! could, the answer is `inconclusive`, which is the expected outcome at these
//! traffic levels and not a failure.
//!
//! Every function here is pure. The synthetic-series tests at the bottom are
//! the real specification: a flat series must not read as `helped`, and a large
//! lift on high traffic must.

use super::metrics::{MetricWindow, TargetDir, TargetMetric};
use serde::{Deserialize, Serialize};

/// Two-sided 95% normal quantile.
const Z: f64 = 1.96;

/// Standard-deviation inflation over the Poisson ideal.
///
/// Web traffic is not Poisson: a single share, a referral spike, or bot traffic
/// the `is_bot` filter missed all cluster arrivals. The proposal's published
/// table is generated with exactly this constant — `Z * 1.5 * sqrt(2n) / n`
/// reproduces its 66/42/24/13/8% row for row, which
/// `reproduces_the_proposals_published_power_table` asserts. Changing it
/// silently re-scales every verdict the feature will ever emit.
const OVERDISPERSION: f64 = 1.5;

/// Fewest weeks of history before a sample standard deviation is used at all.
///
/// Below four weeks the sd of the series is dominated by which weeks happened
/// to land in it, and an under-estimate here is the dangerous direction: it
/// lowers the bar and manufactures verdicts.
const MIN_HISTORY_WEEKS: usize = 4;

/// The minimum detectable effect at which "no detectable change" becomes a
/// statement worth making rather than a restatement of low traffic.
///
/// Tied to the proposal's own line — "≳300 views/week — per-project verdicts
/// are meaningful" — and to the row of its table at 300 views, which is 24%.
/// Above this threshold the window could not have detected even a large change,
/// so a null result says nothing about the action and must read `inconclusive`.
pub const INFORMATIVE_MDE: f64 = 0.25;

/// Which floor set the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerBasis {
    /// Counting noise at the observed volume — used when the project has too
    /// little history to measure its own swing, or when its history is calmer
    /// than counting noise alone would predict.
    Poisson,
    /// The project's own week-to-week standard deviation, which was larger.
    History,
}

impl PowerBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Poisson => "poisson",
            Self::History => "history",
        }
    }
}

/// What size of change this window could actually have detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerCheck {
    /// Smallest absolute change distinguishable from normal variation.
    pub threshold_abs: f64,
    /// The same, relative to the before value. `None` when before is zero, at
    /// which point no percentage is defined.
    pub threshold_pct: Option<f64>,
    pub basis: PowerBasis,
    pub weeks_observed: usize,
    /// Sample sd of the weekly history, when there was enough of it.
    pub weekly_sd: Option<f64>,
}

impl PowerCheck {
    /// Could this window have detected a change big enough to be worth acting
    /// on? Distinguishes `no_effect` (a real null) from `inconclusive` (an
    /// unanswered question).
    pub fn is_informative(&self) -> bool {
        self.threshold_pct.is_some_and(|p| p <= INFORMATIVE_MDE)
    }
}

/// Sample standard deviation (n-1). `None` below two points, where it is
/// undefined rather than zero — reporting 0.0 there would claim a perfectly
/// steady project and set the detection bar to nothing.
pub fn sample_sd(xs: &[f64]) -> Option<f64> {
    if xs.len() < 2 {
        return None;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some(var.sqrt())
}

/// Detection threshold for a count metric over a `window_weeks`-long window.
///
/// Takes the larger of two floors, because they fail in opposite directions:
/// counting noise alone under-states a project that spikes, and a short quiet
/// history under-states a project that is about to.
pub fn count_power(before: f64, after: f64, weekly: &[f64], window_weeks: f64) -> PowerCheck {
    let poisson = Z * OVERDISPERSION * (before + after).max(0.0).sqrt();

    let weekly_sd = if weekly.len() >= MIN_HISTORY_WEEKS {
        sample_sd(weekly)
    } else {
        None
    };
    // A total over k weeks has k times the variance of one week, so the
    // difference of two independent k-week totals has 2k times it.
    let historical = weekly_sd.map(|sd| Z * sd * (2.0 * window_weeks).sqrt());

    let (threshold_abs, basis) = match historical {
        Some(h) if h > poisson => (h, PowerBasis::History),
        _ => (poisson, PowerBasis::Poisson),
    };
    PowerCheck {
        threshold_abs,
        threshold_pct: (before > 0.0).then(|| threshold_abs / before),
        basis,
        weeks_observed: weekly.len(),
        weekly_sd,
    }
}

/// Detection threshold for a rate metric (currently only `bounce_rate`).
///
/// A rate is not a count: it is bounded in [0,1] and its noise is set by its
/// denominator, so the Poisson floor does not apply. This is a two-proportion
/// standard error on the pooled rate, carrying the same overdispersion
/// allowance because sessions from one referral wave are not independent draws.
pub fn ratio_power(
    before_value: f64,
    before_n: f64,
    after_value: f64,
    after_n: f64,
    weekly: &[f64],
    window_weeks: f64,
) -> PowerCheck {
    let proportion = if before_n > 0.0 && after_n > 0.0 {
        let pooled = (before_value * before_n + after_value * after_n) / (before_n + after_n);
        let se = (pooled * (1.0 - pooled) * (1.0 / before_n + 1.0 / after_n)).sqrt();
        Z * OVERDISPERSION * se
    } else {
        // No denominator on one side: nothing is distinguishable, so the bar is
        // above the metric's entire range.
        f64::INFINITY
    };

    let weekly_sd = if weekly.len() >= MIN_HISTORY_WEEKS {
        sample_sd(weekly)
    } else {
        None
    };
    // A rate over k weeks is an AVERAGE of weekly rates, not a sum, so its sd
    // shrinks as sqrt(k) instead of growing — the opposite of `count_power`.
    let historical = weekly_sd.map(|sd| Z * sd * (2.0 / window_weeks).sqrt());

    let (threshold_abs, basis) = match historical {
        Some(h) if h > proportion => (h, PowerBasis::History),
        _ => (proportion, PowerBasis::Poisson),
    };
    PowerCheck {
        threshold_abs,
        threshold_pct: (before_value > 0.0).then(|| threshold_abs / before_value),
        basis,
        weeks_observed: weekly.len(),
        weekly_sd,
    }
}

/// A verdict on one window. `Inconclusive` is a first-class member of this set,
/// not an error variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Helped,
    Hindered,
    NoEffect,
    Inconclusive,
    Confounded,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Helped => "helped",
            Self::Hindered => "hindered",
            Self::NoEffect => "no_effect",
            Self::Inconclusive => "inconclusive",
            Self::Confounded => "confounded",
        }
    }

    /// Only confirmed outcomes feed learning (proposal: "Inconclusive results
    /// teach nothing and must not shift future ranking").
    pub fn feeds_learning(self) -> bool {
        matches!(self, Self::Helped | Self::Hindered | Self::NoEffect)
    }
}

/// Another action verified inside the same window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Confounder {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct JudgeInput<'a> {
    pub dir: TargetDir,
    pub before: &'a MetricWindow,
    pub after: &'a MetricWindow,
    /// Weekly history of the target metric ending at the before window's start,
    /// zero-filled (see `metrics::weekly_history`).
    pub weekly: &'a [f64],
    pub confounders: &'a [Confounder],
    /// Set when the before window reaches back past the earliest event the
    /// database holds, which is indistinguishable from a genuinely quiet week.
    pub coverage_gap: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Judgement {
    pub verdict: Verdict,
    /// Change relative to the before value. `None` when before is zero.
    pub delta_pct: Option<f64>,
    /// One sentence, always populated, always carrying the numbers it rests on.
    pub rationale: String,
    pub confounders: Vec<Confounder>,
    pub power: PowerCheck,
}

fn fmt_value(metric: TargetMetric, v: f64) -> String {
    if metric.is_ratio() {
        format!("{:.0}%", v * 100.0)
    } else {
        format!("{v:.0}")
    }
}

fn fmt_pct(p: f64) -> String {
    format!(
        "{}{:.0}%",
        if p >= 0.0 { "+" } else { "-" },
        p.abs() * 100.0
    )
}

fn fmt_threshold_pct(p: f64) -> String {
    format!("{:.0}%", p * 100.0)
}

/// Decide what, if anything, can be said about one window.
///
/// The order of the guards is the design, not an implementation detail:
/// confounding and missing data are reasons a verdict is *unavailable*, and
/// they must be checked before the arithmetic that would otherwise produce a
/// confident-looking number from them.
///
/// The word "caused" never appears in a rationale. This is a before/after with
/// an honesty gate, not an experiment (proposal, "Non-goals").
pub fn judge(input: JudgeInput<'_>) -> Judgement {
    let JudgeInput {
        dir,
        before,
        after,
        weekly,
        confounders,
        coverage_gap,
    } = input;

    let metric = before.metric;
    let name = metric.as_str();
    let days = after.days;
    let window_weeks = (f64::from(days) / 7.0).max(1.0);

    let power = if metric.is_ratio() {
        ratio_power(
            before.value,
            before.denominator,
            after.value,
            after.denominator,
            weekly,
            window_weeks,
        )
    } else {
        count_power(before.value, after.value, weekly, window_weeks)
    };

    let delta = after.value - before.value;
    let delta_pct = (before.value > 0.0).then(|| delta / before.value);

    // 1. Overlapping work. Attribution across simultaneous changes is not
    //    solvable at this traffic, and the proposal's position is that saying so
    //    is the correct answer.
    if !confounders.is_empty() {
        let names = confounders
            .iter()
            .map(|c| format!("\"{}\"", c.title))
            .collect::<Vec<_>>()
            .join(", ");
        return Judgement {
            verdict: Verdict::Confounded,
            delta_pct,
            rationale: format!(
                "{name} went {} to {} over {days} days, but {} ({names}) was also verified inside \
                 this window, so the change cannot be attributed to either one.",
                fmt_value(metric, before.value),
                fmt_value(metric, after.value),
                if confounders.len() == 1 {
                    "another action"
                } else {
                    "other actions"
                },
            ),
            confounders: confounders.to_vec(),
            power,
        };
    }

    // 2. The before window is partly outside what the database ever recorded.
    if let Some(gap) = coverage_gap {
        return Judgement {
            verdict: Verdict::Inconclusive,
            delta_pct,
            rationale: gap,
            confounders: Vec::new(),
            power,
        };
    }

    // 3. Nothing to compare against. A percentage change from zero is not a
    //    large improvement, it is undefined.
    if before.denominator <= 0.0 {
        let what = if metric.is_ratio() {
            "no sessions were recorded"
        } else {
            "nothing was recorded"
        };
        return Judgement {
            verdict: Verdict::Inconclusive,
            delta_pct: None,
            rationale: format!(
                "{what} for {name} in the {days} days before this action ({} to {}), so there is \
                 no baseline to compare the {} that followed against.",
                before.start,
                before.end,
                fmt_value(metric, after.value),
            ),
            confounders: Vec::new(),
            power,
        };
    }

    // 3b. A rate can be exactly zero while its denominator is healthy (0% bounce
    //     over 400 sessions). There is a baseline, but no percentage change
    //     against it, and every rationale below quotes one.
    let (Some(delta_pct_value), Some(threshold_pct)) = (delta_pct, power.threshold_pct) else {
        return Judgement {
            verdict: Verdict::Inconclusive,
            delta_pct: None,
            rationale: format!(
                "{name} was {} across the {days} days before this action, so a relative change \
                 against it is undefined and the {} that followed cannot be scored.",
                fmt_value(metric, before.value),
                fmt_value(metric, after.value),
            ),
            confounders: Vec::new(),
            power,
        };
    };

    let moved = format!(
        "{name} went {} to {} over {days} days ({})",
        fmt_value(metric, before.value),
        fmt_value(metric, after.value),
        fmt_pct(delta_pct_value),
    );
    let bar = fmt_threshold_pct(threshold_pct);
    let basis = match power.basis {
        PowerBasis::History => format!(
            "this project's own week-to-week swing over {} weeks",
            power.weeks_observed
        ),
        PowerBasis::Poisson => "normal variation at this traffic".to_string(),
    };

    // 4. Big enough to stand out from the project's own noise — AND on enough
    //    traffic for that comparison to mean anything.
    //
    // The volume floor is not redundant with the threshold. The threshold is
    // purely RELATIVE, so it is satisfied at any scale: a project running ~6
    // pageviews a week clears it with 19 extra views and earns
    // "helped, +317%". Nineteen pageviews is one person with a browser open,
    // or one bot session `is_bot` missed — and that verdict then feeds
    // `category_history` and reorders future advice, which is precisely the
    // "training future strategy on noise" the design forbids.
    //
    // `is_informative()` already encodes "this window could detect an effect
    // worth acting on", but it gated only the null branch: a project could be
    // too small to distinguish no_effect from inconclusive, yet still be
    // allowed to shout HELPED. A verdict deserves at least the evidentiary bar
    // we demand before accepting a null.
    if delta.abs() >= power.threshold_abs && power.is_informative() {
        let (verdict, tail) = if dir.agrees_with(delta) {
            (
                Verdict::Helped,
                format!("in the pre-registered direction ({})", dir.as_str()),
            )
        } else {
            (
                Verdict::Hindered,
                format!("against the pre-registered direction ({})", dir.as_str()),
            )
        };
        return Judgement {
            verdict,
            delta_pct,
            rationale: format!("{moved}, past the {bar} that {basis} explains, and {tail}."),
            confounders: Vec::new(),
            power,
        };
    }

    // 5. Too small to stand out — but only a real null when the window had the
    //    power to have seen something worth acting on.
    if power.is_informative() {
        return Judgement {
            verdict: Verdict::NoEffect,
            delta_pct,
            rationale: format!(
                "{moved}, within the {bar} that {basis} explains; this project's traffic was high \
                 enough to have shown a larger change, so there is no detectable effect."
            ),
            confounders: Vec::new(),
            power,
        };
    }

    Judgement {
        verdict: Verdict::Inconclusive,
        delta_pct,
        rationale: format!(
            "{moved}; at this traffic only a change past {bar} stands out from {basis}, so this \
             result is indistinguishable from normal variation."
        ),
        confounders: Vec::new(),
        power,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(metric: TargetMetric, days: u32, value: f64, denominator: f64) -> MetricWindow {
        MetricWindow {
            start: "2026-07-01".into(),
            end: "2026-07-08".into(),
            days,
            metric,
            value,
            denominator,
            pageviews: value as i64,
            sessions: denominator as i64,
        }
    }

    fn judged(
        dir: TargetDir,
        before: &MetricWindow,
        after: &MetricWindow,
        weekly: &[f64],
    ) -> Judgement {
        judge(JudgeInput {
            dir,
            before,
            after,
            weekly,
            confounders: &[],
            coverage_gap: None,
        })
    }

    /// Adversarial review, 2026-08-14: the threshold is purely RELATIVE, so it
    /// is satisfied at any scale. A project on ~6 pageviews a week cleared it
    /// with 19 extra views and earned "helped, +317%" — nineteen pageviews is
    /// one person with a browser open, or one bot `is_bot` missed. That verdict
    /// then fed category_history and reordered future advice, which is exactly
    /// the "training future strategy on noise" the design forbids.
    ///
    /// The reviewer's arithmetic, reproduced: before=6, after=25,
    /// threshold_abs=16.37, delta=19 -> passed branch 4 before the fix.
    #[test]
    fn a_tiny_project_cannot_earn_a_verdict_however_large_the_percentage() {
        for (before_v, after_v, weekly_level) in
            [(6.0, 25.0, 6.0), (3.0, 20.0, 3.0), (1.0, 12.0, 1.0)]
        {
            let weekly = vec![weekly_level; 12];
            let before = window(TargetMetric::Pageviews, 7, before_v, before_v);
            let after = window(TargetMetric::Pageviews, 7, after_v, after_v);
            let j = judged(TargetDir::Up, &before, &after, &weekly);
            assert_ne!(
                j.verdict,
                Verdict::Helped,
                "{before_v} -> {after_v} must not be a verdict: {}",
                j.rationale
            );
            assert_eq!(
                j.verdict,
                Verdict::Inconclusive,
                "and the honest answer is inconclusive: {}",
                j.rationale
            );
        }
    }

    /// The floor must not silence a project with real traffic — that would trade
    /// one dishonesty for another.
    #[test]
    fn a_real_lift_on_real_traffic_still_reads_as_helped() {
        let weekly = vec![1000.0; 12];
        let before = window(TargetMetric::Pageviews, 7, 1000.0, 1000.0);
        let after = window(TargetMetric::Pageviews, 7, 1400.0, 1400.0);
        let j = judged(TargetDir::Up, &before, &after, &weekly);
        assert_eq!(j.verdict, Verdict::Helped, "{}", j.rationale);
    }

    /// A steady series with a steady value must never read as an effect, at any
    /// traffic level. This is the failure mode the whole feature is built to
    /// avoid, so it is asserted across three orders of magnitude.
    #[test]
    fn a_flat_series_never_reads_as_helped() {
        for level in [40.0, 300.0, 1000.0, 5000.0] {
            let weekly = vec![level; 12];
            let before = window(TargetMetric::Pageviews, 7, level, level);
            // A 2% wobble, which is what a flat project looks like in practice.
            let after = window(TargetMetric::Pageviews, 7, level * 1.02, level * 1.02);
            let j = judged(TargetDir::Up, &before, &after, &weekly);
            assert_ne!(j.verdict, Verdict::Helped, "{level}: {}", j.rationale);
            assert_ne!(j.verdict, Verdict::Hindered, "{level}: {}", j.rationale);
        }
    }

    /// The counterpart: a real, large lift on traffic that can carry a verdict
    /// must actually produce one, or the gate is just a mute button.
    #[test]
    fn a_large_lift_on_high_traffic_reads_as_helped() {
        let weekly = vec![
            1000.0, 1030.0, 980.0, 1010.0, 995.0, 1020.0, 1005.0, 990.0, 1015.0, 1000.0, 1025.0,
            985.0,
        ];
        let before = window(TargetMetric::Pageviews, 7, 1000.0, 1000.0);
        let after = window(TargetMetric::Pageviews, 7, 1400.0, 1400.0);
        let j = judged(TargetDir::Up, &before, &after, &weekly);
        assert_eq!(j.verdict, Verdict::Helped, "{}", j.rationale);
        assert!((j.delta_pct.unwrap() - 0.4).abs() < 1e-9);
        assert!(j.rationale.contains("1000 to 1400"), "{}", j.rationale);
        assert!(j.rationale.contains("+40%"), "{}", j.rationale);
    }

    /// The same +40% on 40 views a week is noise, and must be refused. Same
    /// relative delta, opposite verdict — this pair is the whole argument for
    /// computing the threshold per project instead of hardcoding one.
    #[test]
    fn the_same_lift_on_low_traffic_is_inconclusive() {
        let weekly = vec![40.0, 44.0, 36.0, 42.0, 38.0, 41.0, 39.0, 40.0];
        let before = window(TargetMetric::Pageviews, 7, 40.0, 40.0);
        let after = window(TargetMetric::Pageviews, 7, 56.0, 56.0);
        let j = judged(TargetDir::Up, &before, &after, &weekly);
        assert_eq!(j.verdict, Verdict::Inconclusive, "{}", j.rationale);
        assert!(j.rationale.contains("40 to 56"), "{}", j.rationale);
        assert!(j.rationale.contains("indistinguishable"), "{}", j.rationale);
    }

    /// A project whose own history is far noisier than counting noise sets its
    /// own, higher bar. Same numbers as the high-traffic test above, but on a
    /// series that routinely swings ±40% — so the same +40% says nothing.
    #[test]
    fn a_volatile_project_raises_its_own_bar() {
        let volatile = vec![
            1000.0, 1600.0, 500.0, 1400.0, 620.0, 1500.0, 700.0, 1450.0, 560.0, 1380.0, 800.0,
            1200.0,
        ];
        let before = window(TargetMetric::Pageviews, 7, 1000.0, 1000.0);
        let after = window(TargetMetric::Pageviews, 7, 1400.0, 1400.0);
        let j = judged(TargetDir::Up, &before, &after, &volatile);
        assert_eq!(j.verdict, Verdict::Inconclusive, "{}", j.rationale);
        assert_eq!(j.power.basis, PowerBasis::History);
        assert!(
            j.rationale.contains("week-to-week swing over 12 weeks"),
            "{}",
            j.rationale
        );
    }

    /// A well-powered null is a different claim from an underpowered one, and
    /// the two must not collapse into the same word.
    #[test]
    fn a_steady_high_traffic_null_is_no_effect_not_inconclusive() {
        let weekly = vec![3000.0; 12];
        let before = window(TargetMetric::Pageviews, 28, 12000.0, 12000.0);
        let after = window(TargetMetric::Pageviews, 28, 12100.0, 12100.0);
        let j = judged(TargetDir::Up, &before, &after, &weekly);
        assert_eq!(j.verdict, Verdict::NoEffect, "{}", j.rationale);
        assert!(j.power.is_informative());
        assert!(
            j.rationale.contains("high enough to have shown"),
            "{}",
            j.rationale
        );
    }

    #[test]
    fn a_large_drop_against_the_claim_reads_as_hindered() {
        let weekly = vec![1000.0; 12];
        let before = window(TargetMetric::Pageviews, 7, 1000.0, 1000.0);
        let after = window(TargetMetric::Pageviews, 7, 500.0, 500.0);
        let j = judged(TargetDir::Up, &before, &after, &weekly);
        assert_eq!(j.verdict, Verdict::Hindered, "{}", j.rationale);
        assert!(
            j.rationale
                .contains("against the pre-registered direction (up)"),
            "{}",
            j.rationale
        );
    }

    /// A drop is only good news if the action said it would drop. Same numbers
    /// as above with the pre-registered direction flipped.
    #[test]
    fn direction_is_read_from_the_pre_registration_not_the_result() {
        let weekly = vec![1000.0; 12];
        let before = window(TargetMetric::Pageviews, 7, 1000.0, 1000.0);
        let after = window(TargetMetric::Pageviews, 7, 500.0, 500.0);
        assert_eq!(
            judged(TargetDir::Down, &before, &after, &weekly).verdict,
            Verdict::Helped
        );
    }

    #[test]
    fn an_overlapping_action_confounds_the_window_and_is_named() {
        let weekly = vec![1000.0; 12];
        let before = window(TargetMetric::Pageviews, 28, 1000.0, 1000.0);
        let after = window(TargetMetric::Pageviews, 28, 1400.0, 1400.0);
        let j = judge(JudgeInput {
            dir: TargetDir::Up,
            before: &before,
            after: &after,
            weekly: &weekly,
            confounders: &[Confounder {
                id: "a2".into(),
                title: "Rewrite hero copy".into(),
            }],
            coverage_gap: None,
        });
        assert_eq!(j.verdict, Verdict::Confounded);
        assert!(
            j.rationale.contains("\"Rewrite hero copy\""),
            "{}",
            j.rationale
        );
        assert!(
            j.rationale.contains("cannot be attributed"),
            "{}",
            j.rationale
        );
        assert_eq!(j.confounders.len(), 1);
    }

    /// Confounding is checked before the arithmetic, so a delta that would have
    /// read as a strong result does not leak out as one.
    #[test]
    fn confounding_outranks_a_significant_delta() {
        let weekly = vec![1000.0; 12];
        let before = window(TargetMetric::Pageviews, 7, 1000.0, 1000.0);
        let after = window(TargetMetric::Pageviews, 7, 2000.0, 2000.0);
        let clean = judged(TargetDir::Up, &before, &after, &weekly);
        assert_eq!(clean.verdict, Verdict::Helped);

        let j = judge(JudgeInput {
            dir: TargetDir::Up,
            before: &before,
            after: &after,
            weekly: &weekly,
            confounders: &[Confounder {
                id: "a2".into(),
                title: "Other".into(),
            }],
            coverage_gap: None,
        });
        assert_eq!(j.verdict, Verdict::Confounded);
    }

    #[test]
    fn a_missing_baseline_window_is_inconclusive_not_an_infinite_lift() {
        let weekly = vec![0.0; 12];
        let before = window(TargetMetric::Pageviews, 28, 0.0, 0.0);
        let after = window(TargetMetric::Pageviews, 28, 400.0, 400.0);
        let j = judged(TargetDir::Up, &before, &after, &weekly);
        assert_eq!(j.verdict, Verdict::Inconclusive, "{}", j.rationale);
        assert!(j.delta_pct.is_none());
        assert!(j.rationale.contains("no baseline"), "{}", j.rationale);
    }

    #[test]
    fn a_before_window_outside_recorded_history_is_refused() {
        let weekly = vec![100.0; 12];
        let before = window(TargetMetric::Pageviews, 28, 100.0, 100.0);
        let after = window(TargetMetric::Pageviews, 28, 900.0, 900.0);
        let j = judge(JudgeInput {
            dir: TargetDir::Up,
            before: &before,
            after: &after,
            weekly: &weekly,
            confounders: &[],
            coverage_gap: Some(
                "the 28 days before this action start on 2026-01-02, before the \
                                first traffic recorded (2026-02-10)"
                    .into(),
            ),
        });
        assert_eq!(j.verdict, Verdict::Inconclusive);
        assert!(j.rationale.contains("2026-02-10"), "{}", j.rationale);
    }

    /// bounce_rate is a rate: its noise comes from the session denominator, not
    /// from the size of the rate. Six percentage points off 30 sessions is
    /// nothing; the same six points off 3000 is a result.
    #[test]
    fn a_rate_is_judged_against_its_session_denominator() {
        let weekly = vec![0.70, 0.72, 0.69, 0.71, 0.70, 0.73, 0.68, 0.71];
        let thin_before = window(TargetMetric::BounceRate, 7, 0.72, 30.0);
        let thin_after = window(TargetMetric::BounceRate, 7, 0.66, 32.0);
        let thin = judged(TargetDir::Down, &thin_before, &thin_after, &weekly);
        assert_eq!(thin.verdict, Verdict::Inconclusive, "{}", thin.rationale);
        assert!(thin.rationale.contains("72% to 66%"), "{}", thin.rationale);

        let fat_before = window(TargetMetric::BounceRate, 7, 0.72, 3000.0);
        let fat_after = window(TargetMetric::BounceRate, 7, 0.66, 3100.0);
        let fat = judged(TargetDir::Down, &fat_before, &fat_after, &weekly);
        assert_eq!(fat.verdict, Verdict::Helped, "{}", fat.rationale);
    }

    #[test]
    fn a_rate_with_no_sessions_has_no_baseline() {
        let before = window(TargetMetric::BounceRate, 7, 0.0, 0.0);
        let after = window(TargetMetric::BounceRate, 7, 0.5, 20.0);
        let j = judged(TargetDir::Down, &before, &after, &[]);
        assert_eq!(j.verdict, Verdict::Inconclusive, "{}", j.rationale);
        assert!(j.rationale.contains("no sessions"), "{}", j.rationale);
    }

    /// The proposal publishes a table of minimum detectable change by weekly
    /// volume. It is the intuition, not the implementation — but if the code
    /// and the published table disagree, one of them is lying to the user.
    #[test]
    fn reproduces_the_proposals_published_power_table() {
        // (weekly views, published min detectable change)
        for (n, expected) in [
            (40.0, 0.66),
            (100.0, 0.42),
            (300.0, 0.24),
            (1000.0, 0.13),
            (3000.0, 0.08),
        ] {
            let p = count_power(n, n, &[], 1.0);
            let got = p.threshold_pct.unwrap();
            assert!(
                (got - expected).abs() < 0.006,
                "{n} views/week: table says {expected}, code says {got:.4}"
            );
        }
    }

    /// Too little history means the sample sd is noise, so it is not used at
    /// all and the Poisson floor stands alone.
    #[test]
    fn a_short_history_is_not_used_as_a_variance_estimate() {
        let p = count_power(1000.0, 1000.0, &[1000.0, 1001.0, 999.0], 1.0);
        assert_eq!(p.basis, PowerBasis::Poisson);
        assert!(p.weekly_sd.is_none());
        assert_eq!(p.weeks_observed, 3);
    }

    /// A calm history must never LOWER the bar below counting noise — that is
    /// the direction that manufactures verdicts.
    #[test]
    fn a_suspiciously_calm_history_does_not_lower_the_bar() {
        let calm = count_power(1000.0, 1000.0, &[1000.0; 12], 1.0);
        let bare = count_power(1000.0, 1000.0, &[], 1.0);
        assert_eq!(calm.basis, PowerBasis::Poisson);
        assert!((calm.threshold_abs - bare.threshold_abs).abs() < 1e-9);
    }

    #[test]
    fn sample_sd_is_undefined_below_two_points() {
        assert!(sample_sd(&[]).is_none());
        assert!(sample_sd(&[5.0]).is_none());
        assert_eq!(sample_sd(&[1.0, 3.0]), Some(f64::sqrt(2.0)));
    }

    /// Non-goal, stated in the proposal: this is a before/after with an honesty
    /// gate, not an experiment. Every rationale the judge can emit is checked,
    /// because one slipped "caused" is a causal claim the data cannot support.
    #[test]
    fn no_rationale_ever_claims_causation() {
        let weekly_flat = vec![1000.0; 12];
        let weekly_thin = vec![40.0; 12];
        let cases: Vec<Judgement> = vec![
            judged(
                TargetDir::Up,
                &window(TargetMetric::Pageviews, 7, 1000.0, 1000.0),
                &window(TargetMetric::Pageviews, 7, 1400.0, 1400.0),
                &weekly_flat,
            ),
            judged(
                TargetDir::Up,
                &window(TargetMetric::Pageviews, 7, 1000.0, 1000.0),
                &window(TargetMetric::Pageviews, 7, 500.0, 500.0),
                &weekly_flat,
            ),
            judged(
                TargetDir::Up,
                &window(TargetMetric::Pageviews, 28, 12000.0, 12000.0),
                &window(TargetMetric::Pageviews, 28, 12100.0, 12100.0),
                &[3000.0; 12],
            ),
            judged(
                TargetDir::Up,
                &window(TargetMetric::Pageviews, 7, 40.0, 40.0),
                &window(TargetMetric::Pageviews, 7, 56.0, 56.0),
                &weekly_thin,
            ),
            judged(
                TargetDir::Up,
                &window(TargetMetric::Pageviews, 7, 0.0, 0.0),
                &window(TargetMetric::Pageviews, 7, 56.0, 56.0),
                &weekly_thin,
            ),
            judge(JudgeInput {
                dir: TargetDir::Up,
                before: &window(TargetMetric::Pageviews, 7, 1000.0, 1000.0),
                after: &window(TargetMetric::Pageviews, 7, 1400.0, 1400.0),
                weekly: &weekly_flat,
                confounders: &[Confounder {
                    id: "x".into(),
                    title: "Other".into(),
                }],
                coverage_gap: None,
            }),
        ];
        // Every verdict variant must be represented, or this proves nothing.
        let seen: Vec<Verdict> = cases.iter().map(|j| j.verdict).collect();
        for v in [
            Verdict::Helped,
            Verdict::Hindered,
            Verdict::NoEffect,
            Verdict::Inconclusive,
            Verdict::Confounded,
        ] {
            assert!(seen.contains(&v), "verdict {v:?} unreachable: {seen:?}");
        }
        for j in &cases {
            let lower = j.rationale.to_ascii_lowercase();
            assert!(!lower.contains("caus"), "causal claim: {}", j.rationale);
            assert!(!j.rationale.is_empty());
            // Every rationale states the numbers it rests on.
            assert!(
                j.rationale.chars().any(|c| c.is_ascii_digit()),
                "no numbers in: {}",
                j.rationale
            );
        }
    }

    #[test]
    fn only_confirmed_outcomes_feed_learning() {
        assert!(Verdict::Helped.feeds_learning());
        assert!(Verdict::Hindered.feeds_learning());
        assert!(Verdict::NoEffect.feeds_learning());
        assert!(!Verdict::Inconclusive.feeds_learning());
        assert!(!Verdict::Confounded.feeds_learning());
    }
}
