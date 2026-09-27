//! Reject confounded capability samples without changing operational metrics.
//! A failed oracle is useful negative evidence; an unavailable provider or a
//! failed oracle execution is not. Reject the entire sample, rather than drop
//! its difficult tasks and accidentally inflate its pass rate.

pub(crate) fn capability_sample_error(
    provider_unavailable: bool,
    tasks: impl IntoIterator<Item = (bool, bool, bool)>,
) -> Option<&'static str> {
    if provider_unavailable {
        return Some("provider unavailable");
    }
    for (provider_error, execution_error, oracle_timed_out) in tasks {
        if provider_error {
            return Some("task provider unavailable");
        }
        if execution_error || oracle_timed_out {
            return Some("task oracle execution incomplete or errored");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_outage_invalidates_even_a_pass_looking_sample() {
        assert!(capability_sample_error(true, [(false, false, false)]).is_some());
    }

    #[test]
    fn task_outage_does_not_depend_on_record_summary() {
        assert!(capability_sample_error(false, [(true, false, false)]).is_some());
    }

    #[test]
    fn mixed_sample_is_rejected_instead_of_filtering_the_failed_task() {
        assert!(
            capability_sample_error(false, [(false, false, false), (true, true, false)]).is_some()
        );
    }

    #[test]
    fn execution_errors_and_killed_oracles_are_not_capability_failures() {
        assert!(capability_sample_error(false, [(false, true, false)]).is_some());
        assert!(capability_sample_error(false, [(false, false, true)]).is_some());
    }

    #[test]
    fn genuine_oracle_passes_and_failures_remain_gradeable() {
        assert_eq!(
            capability_sample_error(false, [(false, false, false); 2]),
            None
        );
    }
}
