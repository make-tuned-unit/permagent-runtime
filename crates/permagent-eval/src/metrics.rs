//! Aggregation: pass-rate, $/solved, and median $/task, with the awkward edge
//! cases (0 solved, all solved, unknown / estimated costs) handled explicitly.
//!
//! All functions here are pure over [`TaskResult`] slices — no I/O — and are the
//! most heavily tested part of the harness, since these numbers are the whole
//! point of the eval.

use crate::cost::CostReading;
use crate::oracle::OracleOutcome;

/// The outcome of running one task under one tier.
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub task_id: String,
    pub category: String,
    /// Whether the oracle passed.
    pub solved: bool,
    /// Cost as read from the run's ledger.
    pub cost: CostReading,
    /// Wall-clock seconds the harness run took.
    pub duration_secs: f64,
    /// The harness process exit code (`None` = killed / timed out / spawn error).
    pub harness_exit: Option<i32>,
    /// Whether the harness run hit its wall-clock ceiling.
    pub harness_timed_out: bool,
    /// The oracle verdict.
    pub oracle: OracleOutcome,
    /// Optional human note (e.g. an error explanation).
    pub note: Option<String>,
}

impl TaskResult {
    /// Convenience constructor used by the runner and tests.
    pub fn new(
        task_id: impl Into<String>,
        category: impl Into<String>,
        oracle: OracleOutcome,
        cost: CostReading,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            category: category.into(),
            solved: oracle.solved(),
            cost,
            duration_secs: 0.0,
            harness_exit: None,
            harness_timed_out: false,
            oracle,
            note: None,
        }
    }
}

/// Aggregated metrics for one tier's run over the task set.
#[derive(Debug, Clone, PartialEq)]
pub struct Aggregate {
    pub total: usize,
    pub solved: usize,
    /// Fraction solved in `[0, 1]` (0.0 for an empty set).
    pub pass_rate: f64,
    /// Sum of known per-task costs, or `None` if no task had a known cost.
    pub total_cost_usd: Option<f64>,
    /// True if at least one task's cost was unknown (so totals are a lower bound).
    pub any_cost_unknown: bool,
    /// True if any task's cost was flagged estimated (a further under-count).
    pub any_estimated: bool,
    /// Total known cost divided by number solved — the headline "$/solved".
    /// `None` when nothing was solved, or when no cost is known.
    pub dollars_per_solved: Option<f64>,
    /// Median of the known per-task costs. `None` when no cost is known.
    pub median_cost_per_task: Option<f64>,
}

/// Fraction of tasks solved, in `[0, 1]`. An empty set is `0.0`.
pub fn pass_rate(results: &[TaskResult]) -> f64 {
    if results.is_empty() {
        return 0.0;
    }
    let solved = results.iter().filter(|r| r.solved).count();
    solved as f64 / results.len() as f64
}

/// Sum of the known per-task costs. Returns `(total, any_unknown, any_estimated)`
/// where `total` is `None` only when *no* task had a known cost.
pub fn total_cost(results: &[TaskResult]) -> (Option<f64>, bool, bool) {
    let mut sum = 0.0;
    let mut any_known = false;
    let mut any_unknown = false;
    let mut any_estimated = false;
    for r in results {
        match r.cost.usd {
            Some(u) => {
                sum += u;
                any_known = true;
            }
            None => any_unknown = true,
        }
        if r.cost.estimated {
            any_estimated = true;
        }
    }
    (any_known.then_some(sum), any_unknown, any_estimated)
}

/// The headline cost-efficiency metric: total known spend per solved task. `None`
/// when nothing was solved or no cost is known. Note the numerator is spend over
/// *all* tasks, so failed attempts are amortised into the price of a solve.
pub fn dollars_per_solved(results: &[TaskResult]) -> Option<f64> {
    let solved = results.iter().filter(|r| r.solved).count();
    if solved == 0 {
        return None;
    }
    let (total, _, _) = total_cost(results);
    total.map(|t| t / solved as f64)
}

/// Median of the known per-task costs (`None` when none are known). For an even
/// count it is the mean of the two middle values.
pub fn median_cost_per_task(results: &[TaskResult]) -> Option<f64> {
    let mut costs: Vec<f64> = results.iter().filter_map(|r| r.cost.usd).collect();
    if costs.is_empty() {
        return None;
    }
    costs.sort_by(f64::total_cmp);
    let n = costs.len();
    let mid = n / 2;
    if n % 2 == 1 {
        Some(costs[mid])
    } else {
        Some((costs[mid - 1] + costs[mid]) / 2.0)
    }
}

/// Compute the full [`Aggregate`] for a tier's results.
pub fn aggregate(results: &[TaskResult]) -> Aggregate {
    let (total_cost_usd, any_cost_unknown, any_estimated) = total_cost(results);
    Aggregate {
        total: results.len(),
        solved: results.iter().filter(|r| r.solved).count(),
        pass_rate: pass_rate(results),
        total_cost_usd,
        any_cost_unknown,
        any_estimated,
        dollars_per_solved: dollars_per_solved(results),
        median_cost_per_task: median_cost_per_task(results),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(id: &str, solved: bool, cost: Option<f64>) -> TaskResult {
        let oracle = if solved {
            OracleOutcome::Pass
        } else {
            OracleOutcome::Fail
        };
        let cost = match cost {
            Some(u) => CostReading::known(u, false, 1),
            None => CostReading::unknown(),
        };
        TaskResult::new(id, "classic", oracle, cost)
    }

    #[test]
    fn pass_rate_basics() {
        assert_eq!(pass_rate(&[]), 0.0);
        let all = vec![res("a", true, Some(1.0)), res("b", true, Some(1.0))];
        assert_eq!(pass_rate(&all), 1.0);
        let none = vec![res("a", false, Some(1.0)), res("b", false, Some(1.0))];
        assert_eq!(pass_rate(&none), 0.0);
        let mixed = vec![
            res("a", true, Some(1.0)),
            res("b", false, Some(1.0)),
            res("c", true, Some(1.0)),
            res("d", false, Some(1.0)),
        ];
        assert_eq!(pass_rate(&mixed), 0.5);
    }

    #[test]
    fn dollars_per_solved_zero_solved_is_none() {
        let r = vec![res("a", false, Some(1.0)), res("b", false, Some(2.0))];
        assert_eq!(dollars_per_solved(&r), None);
    }

    #[test]
    fn dollars_per_solved_amortises_failures() {
        // total spend 3.0 across 2 solves => 1.5 per solve, even though one task failed.
        let r = vec![
            res("a", true, Some(1.0)),
            res("b", true, Some(1.0)),
            res("c", false, Some(1.0)),
        ];
        assert_eq!(dollars_per_solved(&r), Some(1.5));
    }

    #[test]
    fn dollars_per_solved_all_solved() {
        let r = vec![res("a", true, Some(2.0)), res("b", true, Some(4.0))];
        assert_eq!(dollars_per_solved(&r), Some(3.0));
    }

    #[test]
    fn median_odd_even_and_single() {
        assert_eq!(
            median_cost_per_task(&[res("a", true, Some(5.0))]),
            Some(5.0)
        );
        let odd = vec![
            res("a", true, Some(3.0)),
            res("b", true, Some(1.0)),
            res("c", true, Some(2.0)),
        ];
        assert_eq!(median_cost_per_task(&odd), Some(2.0));
        let even = vec![
            res("a", true, Some(1.0)),
            res("b", true, Some(2.0)),
            res("c", true, Some(3.0)),
            res("d", true, Some(4.0)),
        ];
        assert_eq!(median_cost_per_task(&even), Some(2.5));
    }

    #[test]
    fn median_and_total_none_when_all_costs_unknown() {
        let r = vec![res("a", true, None), res("b", false, None)];
        assert_eq!(median_cost_per_task(&r), None);
        let (total, any_unknown, _) = total_cost(&r);
        assert_eq!(total, None);
        assert!(any_unknown);
    }

    #[test]
    fn total_cost_sums_known_and_flags_unknown() {
        let r = vec![
            res("a", true, Some(1.0)),
            res("b", true, None),
            res("c", true, Some(0.5)),
        ];
        let (total, any_unknown, _) = total_cost(&r);
        assert_eq!(total, Some(1.5));
        assert!(any_unknown);
    }

    #[test]
    fn total_cost_all_free_is_some_zero() {
        let r = vec![res("a", true, Some(0.0)), res("b", true, Some(0.0))];
        let (total, any_unknown, _) = total_cost(&r);
        assert_eq!(total, Some(0.0));
        assert!(!any_unknown);
    }

    #[test]
    fn aggregate_end_to_end_mixed() {
        let r = vec![
            res("a", true, Some(0.10)),
            res("b", false, Some(0.20)),
            res("c", true, Some(0.30)),
        ];
        let agg = aggregate(&r);
        assert_eq!(agg.total, 3);
        assert_eq!(agg.solved, 2);
        assert!((agg.pass_rate - 2.0 / 3.0).abs() < 1e-12);
        assert!((agg.total_cost_usd.unwrap() - 0.60).abs() < 1e-12);
        assert!((agg.dollars_per_solved.unwrap() - 0.30).abs() < 1e-12);
        assert_eq!(agg.median_cost_per_task, Some(0.20));
        assert!(!agg.any_cost_unknown);
    }

    #[test]
    fn aggregate_zero_solved_leaves_dollars_per_solved_none() {
        let r = vec![res("a", false, Some(0.10)), res("b", false, Some(0.20))];
        let agg = aggregate(&r);
        assert_eq!(agg.solved, 0);
        assert_eq!(agg.pass_rate, 0.0);
        assert_eq!(agg.dollars_per_solved, None);
        assert!((agg.total_cost_usd.unwrap() - 0.30).abs() < 1e-12);
    }

    #[test]
    fn aggregate_estimated_flag_propagates() {
        let mut t = res("a", true, Some(0.0));
        t.cost = CostReading::known(0.0, true, 2);
        let agg = aggregate(&[t]);
        assert!(agg.any_estimated);
    }
}
