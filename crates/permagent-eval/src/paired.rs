//! Paired A/B evaluation — does a feature actually help?
//!
//! Built for one question the repo cannot currently answer: **does injecting
//! distilled lessons make the agent better or worse?** It has twice measured
//! ~9 points LOST when hints were fed back as authority (`librarian_atoms`
//! −9.2pp; `playbook` ~−9). So "turn lessons on" must be earned by a
//! measurement, not by hope.
//!
//! ## Why repeats, not just tasks
//!
//! The curated set is small (7 tasks at time of writing). A 9-point effect on 7
//! tasks is **0.6 of a task** — a single paired run cannot distinguish a real
//! effect from one task flipping on model stochasticity. Repeats are how a small
//! task set yields resolvable samples: N runs per task per arm.
//!
//! ## The honesty rule
//!
//! A delta computed from too few runs is worse than no delta, because it looks
//! like permission. [`PairedReport::sample_warning`] fires below
//! [`MIN_RUNS_PER_ARM_FOR_CONFIDENCE`] and says so in plain language, and the
//! report always carries raw counts so a reader can judge for themselves rather
//! than trusting a percentage.

use crate::metrics::TaskResult;

/// Below this many runs per arm, a delta cannot resolve the ~9-point effect
/// this harness exists to test, and the report says so.
///
/// Rationale rather than taste: detecting a ~0.09 difference in two proportions
/// near 0.5 at conventional power needs samples in the hundreds. Thirty is not
/// that — it is the floor below which the number is not worth *reading*, and it
/// is chosen to be honest about a small curated set rather than to imply rigour
/// the sample cannot support.
pub const MIN_RUNS_PER_ARM_FOR_CONFIDENCE: usize = 30;

/// Which side of the comparison a run belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    /// Feature OFF — the baseline.
    Control,
    /// Feature ON.
    Treatment,
}

impl Arm {
    pub fn label(self) -> &'static str {
        match self {
            Arm::Control => "control (off)",
            Arm::Treatment => "treatment (on)",
        }
    }
}

/// One arm's outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmSummary {
    pub arm_label: String,
    pub runs: usize,
    pub solved: usize,
    pub pass_rate: f64,
}

impl ArmSummary {
    pub fn from_results(arm: Arm, results: &[TaskResult]) -> Self {
        let solved = results.iter().filter(|r| r.solved).count();
        ArmSummary {
            arm_label: arm.label().to_string(),
            runs: results.len(),
            solved,
            pass_rate: if results.is_empty() {
                0.0
            } else {
                solved as f64 / results.len() as f64
            },
        }
    }
}

/// The comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct PairedReport {
    pub control: ArmSummary,
    pub treatment: ArmSummary,
    /// treatment − control, in pass-rate points. Positive means the feature helped.
    pub delta: f64,
    /// Present when the sample is too small for the delta to mean anything.
    pub sample_warning: Option<String>,
}

/// Compare two arms.
pub fn compare(control: &[TaskResult], treatment: &[TaskResult]) -> PairedReport {
    let c = ArmSummary::from_results(Arm::Control, control);
    let t = ArmSummary::from_results(Arm::Treatment, treatment);
    let delta = t.pass_rate - c.pass_rate;

    let smallest = c.runs.min(t.runs);
    let sample_warning = if smallest < MIN_RUNS_PER_ARM_FOR_CONFIDENCE {
        Some(format!(
            "SAMPLE TOO SMALL — {} run(s) in the smaller arm, under the {} this comparison needs. \
             A {:+.1} point delta here is within the noise of a single task flipping, and must \
             NOT be read as evidence the feature helps or hurts. Raise --repeats or add tasks \
             before acting on this number.",
            smallest,
            MIN_RUNS_PER_ARM_FOR_CONFIDENCE,
            delta * 100.0
        ))
    } else {
        None
    };

    PairedReport {
        control: c,
        treatment: t,
        delta,
        sample_warning,
    }
}

/// Render the comparison. Counts always accompany the percentage — a bare
/// percentage on a small n is how a thin result gets mistaken for a green light.
pub fn format_report(r: &PairedReport) -> String {
    let mut out = String::from("\nPaired A/B result\n");
    out.push_str(&format!(
        "  {:<16} {:>3}/{:<3} solved  ({:.1}%)\n",
        r.control.arm_label,
        r.control.solved,
        r.control.runs,
        r.control.pass_rate * 100.0
    ));
    out.push_str(&format!(
        "  {:<16} {:>3}/{:<3} solved  ({:.1}%)\n",
        r.treatment.arm_label,
        r.treatment.solved,
        r.treatment.runs,
        r.treatment.pass_rate * 100.0
    ));
    out.push_str(&format!(
        "  delta            {:+.1} points\n",
        r.delta * 100.0
    ));
    if let Some(w) = &r.sample_warning {
        out.push_str(&format!("\n  ⚠ {w}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cost::CostReading;
    use crate::oracle::OracleOutcome;

    fn results(total: usize, solved: usize) -> Vec<TaskResult> {
        (0..total)
            .map(|i| {
                let outcome = if i < solved {
                    OracleOutcome::Pass
                } else {
                    OracleOutcome::Fail
                };
                TaskResult::new("t", "regression", outcome, CostReading::unknown())
            })
            .collect()
    }

    #[test]
    fn delta_is_treatment_minus_control() {
        let r = compare(&results(100, 50), &results(100, 60));
        assert!((r.delta - 0.10).abs() < 1e-9, "expected +10 points");
        assert_eq!(r.control.solved, 50);
        assert_eq!(r.treatment.solved, 60);
    }

    #[test]
    fn a_worse_treatment_reports_a_negative_delta() {
        // The -9.2pp case this harness exists to catch.
        let r = compare(&results(100, 60), &results(100, 51));
        assert!(r.delta < 0.0);
        assert!((r.delta + 0.09).abs() < 1e-9);
    }

    #[test]
    fn a_small_sample_is_flagged_and_says_not_to_act_on_it() {
        // 7 tasks, 1 repeat — exactly the naive run that would mislead.
        let r = compare(&results(7, 3), &results(7, 4));
        let w = r.sample_warning.expect("small sample must be flagged");
        assert!(w.contains("SAMPLE TOO SMALL"));
        assert!(w.contains("must NOT be read as evidence"));
        // The delta is still reported — suppressing it would be its own dishonesty.
        assert!(r.delta > 0.0);
    }

    #[test]
    fn a_sufficient_sample_carries_no_warning() {
        let n = MIN_RUNS_PER_ARM_FOR_CONFIDENCE;
        let r = compare(&results(n, n / 2), &results(n, n / 2));
        assert!(r.sample_warning.is_none());
    }

    #[test]
    fn the_smaller_arm_decides_the_warning() {
        // A large control does not license a tiny treatment.
        let r = compare(&results(200, 100), &results(5, 3));
        assert!(r.sample_warning.is_some());
    }

    #[test]
    fn the_rendered_report_always_shows_raw_counts_next_to_the_percentage() {
        let out = format_report(&compare(&results(7, 3), &results(7, 5)));
        assert!(out.contains("3/7"), "counts must be visible");
        assert!(out.contains("5/7"));
        assert!(out.contains("delta"));
        assert!(out.contains("SAMPLE TOO SMALL"));
    }

    #[test]
    fn empty_arms_do_not_panic_and_are_flagged() {
        let r = compare(&[], &[]);
        assert_eq!(r.delta, 0.0);
        assert!(r.sample_warning.is_some());
    }
}
