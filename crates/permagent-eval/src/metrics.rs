//! Aggregation: pass-rate, $/solved, and median $/task, with the awkward edge
//! cases (0 solved, all solved, unknown / estimated costs) handled explicitly.
//!
//! All functions here are pure over [`TaskResult`] slices — no I/O — and are the
//! most heavily tested part of the harness, since these numbers are the whole
//! point of the eval.

use crate::cost::CostReading;
use crate::harness_log::LogSignals;
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
    /// Optional human note (e.g. an error explanation, or why a task was
    /// skipped for [`OracleOutcome::NotRun`]).
    pub note: Option<String>,
    /// Best-effort signals mined from the harness's captured log (tool calls,
    /// rate-limit events, turn-limit hit). Zeroed for a task that was never
    /// run.
    pub signals: LogSignals,
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
            signals: LogSignals::default(),
        }
    }
}

/// Aggregated metrics for one tier's run over the task set.
#[derive(Debug, Clone, PartialEq)]
pub struct Aggregate {
    /// Every task in the set, including any [`OracleOutcome::NotRun`] ones.
    pub total: usize,
    /// Tasks actually launched: `total - not_run`.
    pub attempted: usize,
    /// Tasks skipped because a `--budget-usd` cap was exceeded before they
    /// could launch (see [`OracleOutcome::NotRun`]).
    pub not_run: usize,
    pub solved: usize,
    /// Fraction solved in `[0, 1]`, over ATTEMPTED tasks only (0.0 when none
    /// were attempted). A not-run task is excluded from both halves of the
    /// ratio, so a budget stop never drags pass-rate down.
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
    /// Median wall-clock seconds across attempted tasks. `None` when none
    /// were attempted.
    pub median_duration_secs: Option<f64>,
    /// Sum of `signals.tool_calls` across every result.
    pub total_tool_calls: usize,
    /// Sum of `signals.rate_limit_events` across every result.
    pub total_rate_limit_events: usize,
    /// Aggregate cache-hit rate: total cache-read tokens over total input
    /// tokens, summed across every task with known token figures. `None`
    /// when no task has known token data. See
    /// [`CostReading::cache_hit_rate`](crate::cost::CostReading::cache_hit_rate)
    /// for the single-task version and its "inclusive input" caveat.
    pub cache_hit_rate: Option<f64>,
}

/// Tasks NOT skipped by a budget stop — i.e. everything except
/// [`OracleOutcome::NotRun`]. The denominator for [`pass_rate`].
pub fn attempted_count(results: &[TaskResult]) -> usize {
    results
        .iter()
        .filter(|r| r.oracle != OracleOutcome::NotRun)
        .count()
}

/// Fraction of ATTEMPTED tasks solved, in `[0, 1]` (0.0 if none were
/// attempted). [`OracleOutcome::NotRun`] tasks are excluded from both the
/// numerator and the denominator — a budget stop must never look like a run
/// of failures.
pub fn pass_rate(results: &[TaskResult]) -> f64 {
    let attempted = attempted_count(results);
    if attempted == 0 {
        return 0.0;
    }
    let solved = results.iter().filter(|r| r.solved).count();
    solved as f64 / attempted as f64
}

/// Median wall-clock seconds across attempted tasks (excludes
/// [`OracleOutcome::NotRun`], which never ran). `None` when none were
/// attempted.
pub fn median_duration_secs(results: &[TaskResult]) -> Option<f64> {
    let mut ds: Vec<f64> = results
        .iter()
        .filter(|r| r.oracle != OracleOutcome::NotRun)
        .map(|r| r.duration_secs)
        .collect();
    if ds.is_empty() {
        return None;
    }
    ds.sort_by(f64::total_cmp);
    let n = ds.len();
    let mid = n / 2;
    if n % 2 == 1 {
        Some(ds[mid])
    } else {
        Some((ds[mid - 1] + ds[mid]) / 2.0)
    }
}

/// Sum of `signals.tool_calls` across every result (not-run tasks contribute 0).
pub fn total_tool_calls(results: &[TaskResult]) -> usize {
    results.iter().map(|r| r.signals.tool_calls).sum()
}

/// Sum of `signals.rate_limit_events` across every result.
pub fn total_rate_limit_events(results: &[TaskResult]) -> usize {
    results.iter().map(|r| r.signals.rate_limit_events).sum()
}

/// Aggregate cache-hit rate across every task with known token figures: total
/// cache-read tokens over total input tokens. `None` when no task carries
/// known token data.
pub fn cache_hit_rate(results: &[TaskResult]) -> Option<f64> {
    let mut total_input: i64 = 0;
    let mut total_cache_read: i64 = 0;
    let mut any_known = false;
    for r in results {
        if let Some(input) = r.cost.input_tokens {
            total_input += input;
            total_cache_read += r.cost.cache_read_tokens.unwrap_or(0);
            any_known = true;
        }
    }
    if !any_known || total_input <= 0 {
        return None;
    }
    Some(total_cache_read as f64 / total_input as f64)
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
    let not_run = results
        .iter()
        .filter(|r| r.oracle == OracleOutcome::NotRun)
        .count();
    Aggregate {
        total: results.len(),
        attempted: results.len() - not_run,
        not_run,
        solved: results.iter().filter(|r| r.solved).count(),
        pass_rate: pass_rate(results),
        total_cost_usd,
        any_cost_unknown,
        any_estimated,
        dollars_per_solved: dollars_per_solved(results),
        median_cost_per_task: median_cost_per_task(results),
        median_duration_secs: median_duration_secs(results),
        total_tool_calls: total_tool_calls(results),
        total_rate_limit_events: total_rate_limit_events(results),
        cache_hit_rate: cache_hit_rate(results),
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

    fn not_run(id: &str) -> TaskResult {
        TaskResult::new(id, "classic", OracleOutcome::NotRun, CostReading::unknown())
    }

    #[test]
    fn pass_rate_excludes_not_run_tasks_from_both_halves_of_the_ratio() {
        // 1 solved, 1 failed, 2 not-run => pass-rate is 1/2 (attempted only),
        // NOT 1/4 (which would let a budget stop look like a run of failures).
        let r = vec![
            res("a", true, Some(1.0)),
            res("b", false, Some(1.0)),
            not_run("c"),
            not_run("d"),
        ];
        assert_eq!(pass_rate(&r), 0.5);
        assert_eq!(attempted_count(&r), 2);
    }

    #[test]
    fn pass_rate_is_zero_when_every_task_is_not_run() {
        let r = vec![not_run("a"), not_run("b")];
        assert_eq!(pass_rate(&r), 0.0);
        assert_eq!(attempted_count(&r), 0);
    }

    #[test]
    fn aggregate_reports_attempted_and_not_run_counts() {
        let r = vec![res("a", true, Some(1.0)), not_run("b"), not_run("c")];
        let agg = aggregate(&r);
        assert_eq!(agg.total, 3);
        assert_eq!(agg.not_run, 2);
        assert_eq!(agg.attempted, 1);
        assert_eq!(agg.solved, 1);
        assert_eq!(agg.pass_rate, 1.0);
    }

    #[test]
    fn median_duration_excludes_not_run_and_handles_odd_even() {
        let mut a = res("a", true, Some(1.0));
        a.duration_secs = 10.0;
        let mut b = res("b", true, Some(1.0));
        b.duration_secs = 20.0;
        let mut c = res("c", true, Some(1.0));
        c.duration_secs = 30.0;
        let skipped = not_run("d"); // duration_secs defaults to 0.0, must be excluded

        assert_eq!(
            median_duration_secs(&[a.clone(), b.clone(), c.clone(), skipped]),
            Some(20.0)
        );
        assert_eq!(median_duration_secs(&[a, b]), Some(15.0));
        assert_eq!(median_duration_secs(&[]), None);
        assert_eq!(median_duration_secs(&[not_run("only")]), None);
    }

    #[test]
    fn total_tool_calls_and_rate_limit_events_sum_across_results() {
        let mut a = res("a", true, Some(1.0));
        a.signals.tool_calls = 3;
        a.signals.rate_limit_events = 1;
        let mut b = res("b", true, Some(1.0));
        b.signals.tool_calls = 5;
        b.signals.rate_limit_events = 0;
        let r = vec![a, b, not_run("c")];
        assert_eq!(total_tool_calls(&r), 8);
        assert_eq!(total_rate_limit_events(&r), 1);
    }

    #[test]
    fn cache_hit_rate_aggregates_across_tasks_with_known_tokens() {
        let mut a = res("a", true, Some(1.0));
        a.cost =
            CostReading::known_with_tokens(1.0, false, 1, Some(1000), Some(100), Some(400), None);
        let mut b = res("b", true, Some(1.0));
        b.cost =
            CostReading::known_with_tokens(1.0, false, 1, Some(500), Some(50), Some(100), None);
        let r = vec![a, b];
        // (400 + 100) / (1000 + 500) = 500/1500
        assert!((cache_hit_rate(&r).unwrap() - 500.0 / 1500.0).abs() < 1e-12);
    }

    #[test]
    fn cache_hit_rate_is_none_when_no_task_has_known_tokens() {
        let r = vec![res("a", true, Some(1.0)), res("b", false, None)];
        assert_eq!(cache_hit_rate(&r), None);
    }
}
