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
}
