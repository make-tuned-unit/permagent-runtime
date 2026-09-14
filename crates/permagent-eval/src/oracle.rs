//! Turning a test-oracle process outcome into pass/fail.
//!
//! The oracle command runs in the finished workspace; its exit status is the sole
//! correctness signal. Exit 0 = solved. Any other exit = failed. No exit code
//! (killed / timed out / spawn error) = errored, which counts as not solved.

/// The result of running a task's test oracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleOutcome {
    /// Exit code 0 — the task is solved.
    Pass,
    /// Non-zero exit — the produced solution is wrong.
    Fail,
    /// The oracle could not produce a verdict (timeout, signal, spawn failure).
    Errored,
    /// The task was never launched — e.g. a `--budget-usd` cap was exceeded by
    /// an earlier task in the session. A distinct state, NOT a fail: it must
    /// not count against pass-rate (whose denominator is attempted tasks
    /// only) and must not be conflated with a real, graded failure.
    NotRun,
}

impl OracleOutcome {
    /// Whether this outcome counts the task as solved (only [`Pass`] does).
    pub fn solved(self) -> bool {
        matches!(self, OracleOutcome::Pass)
    }

    /// A short stable label for reports.
    pub fn label(self) -> &'static str {
        match self {
            OracleOutcome::Pass => "pass",
            OracleOutcome::Fail => "fail",
            OracleOutcome::Errored => "errored",
            OracleOutcome::NotRun => "not_run",
        }
    }
}

/// Map a process exit code (`None` = no code, i.e. killed/timed-out) to an
/// [`OracleOutcome`].
pub fn outcome_from_exit(code: Option<i32>) -> OracleOutcome {
    match code {
        Some(0) => OracleOutcome::Pass,
        Some(_) => OracleOutcome::Fail,
        None => OracleOutcome::Errored,
    }
}

/// Map an oracle process's outcome to [`OracleOutcome`], the same as
/// [`outcome_from_exit`] EXCEPT that `timed_out` always wins.
///
/// Lane K7: [`crate::runner::wait_with_timeout`] sends `SIGTERM` on a
/// wall-clock-ceiling breach and then polls for a grace period before the
/// hard `SIGKILL` fallback — a process that exits during that grace window
/// (having caught the signal, or having simply finished at almost the same
/// instant) still reports a real exit code, e.g. `Some(0)` or `Some(1)`,
/// even though the run objectively hit its ceiling and had to be killed. An
/// oracle whose wall-clock ceiling tripped could not produce a trustworthy
/// verdict either way — grading a build/test process AS it is being torn
/// down is not a graded run — so `timed_out` must force
/// [`OracleOutcome::Errored`] regardless of what exit code (if any) came
/// back, never fall through to [`outcome_from_exit`]'s naive 0/nonzero
/// reading.
pub fn outcome_from_run(code: Option<i32>, timed_out: bool) -> OracleOutcome {
    if timed_out {
        OracleOutcome::Errored
    } else {
        outcome_from_exit(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_pass_and_solved() {
        let o = outcome_from_exit(Some(0));
        assert_eq!(o, OracleOutcome::Pass);
        assert!(o.solved());
    }

    #[test]
    fn nonzero_is_fail_not_solved() {
        let o = outcome_from_exit(Some(1));
        assert_eq!(o, OracleOutcome::Fail);
        assert!(!o.solved());
        assert_eq!(outcome_from_exit(Some(2)), OracleOutcome::Fail);
    }

    #[test]
    fn no_code_is_errored_not_solved() {
        let o = outcome_from_exit(None);
        assert_eq!(o, OracleOutcome::Errored);
        assert!(!o.solved());
        assert_eq!(o.label(), "errored");
    }

    #[test]
    fn not_run_is_not_solved_and_is_distinct_from_fail() {
        let o = OracleOutcome::NotRun;
        assert!(!o.solved());
        assert_eq!(o.label(), "not_run");
        assert_ne!(OracleOutcome::NotRun, OracleOutcome::Fail);
    }

    #[test]
    fn timed_out_is_errored_even_with_a_passing_exit_code() {
        // A process that caught SIGTERM during the grace window and exited
        // 0 right as it was being killed did not produce a trustworthy
        // verdict — the ceiling breach must win.
        assert_eq!(outcome_from_run(Some(0), true), OracleOutcome::Errored);
    }

    #[test]
    fn timed_out_is_errored_not_fail_even_with_a_failing_exit_code() {
        // This is the A17/M20 shape: a wrapper script's own internal alarm
        // truncates a build step (SIGALRM -> exit 142), the script folds
        // that into its own nonzero exit, and a naive exit-code read would
        // call it a real `Fail` rather than the grading failure it is.
        assert_eq!(outcome_from_run(Some(142), true), OracleOutcome::Errored);
        assert_eq!(outcome_from_run(Some(1), true), OracleOutcome::Errored);
    }

    #[test]
    fn not_timed_out_falls_through_to_outcome_from_exit() {
        assert_eq!(outcome_from_run(Some(0), false), OracleOutcome::Pass);
        assert_eq!(outcome_from_run(Some(1), false), OracleOutcome::Fail);
        assert_eq!(outcome_from_run(None, false), OracleOutcome::Errored);
    }
}
