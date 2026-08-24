//! One run row per attempted pass — the seam the daemon's sweep loops share.
//!
//! `permagent::agent_runs` is the durable table; this module is the thin
//! decision layer above it that `strix`, `steward_sweep` and `watcher_insights`
//! actually call. It exists for two reasons none of those loops could solve
//! on its own:
//!
//! * **Exactly one row per invocation.** A sweep has many exits — a gate, a
//!   preflight, an empty rotation, a scope refusal, an egress refusal, an
//!   error, and finally the real work. Sprinkling a `record` call at each of
//!   them by hand is how a path quietly ends up recording twice, or not at
//!   all, and "this pass left no trace" is the precise ambiguity the run table
//!   was added to remove. So each loop's body now RETURNS a [`Pass`] instead of
//!   returning early, and the single `record_pass` call sits above it where it
//!   cannot be missed.
//!
//! * **The record and the loop's `Result` are two different statements.** They
//!   agree most of the time and genuinely disagree in places: the Guard's
//!   preflight failure is a SKIP in the record (the sweep correctly declined to
//!   pretend-scan) while the loop has always returned `Err` so its own debug
//!   line reads "sweep skipped: preflight failed: …"; a scan that errored is a
//!   FAILED pass in the record while the loop has always returned `Ok(())`,
//!   because the rotation advanced and one broken repo is not a broken sweep.
//!   [`Pass::returning`] is where that divergence is stated out loud at the one
//!   call site it applies to, so the run row can be honest without changing a
//!   byte of what the loop does next.
//!
//! ## Where the line is drawn — the same in all three loops
//!
//! A pass records a row only if it was genuinely ATTEMPTED.
//!
//! The "is the feature on" and "is a sweep due yet" checks live in each loop's
//! `spawn`, ABOVE the pass function, and record nothing. A Guard that has been
//! switched off for a month must not accumulate one skipped row every fifteen
//! minutes saying so: that is not evidence of liveness, it is noise deep enough
//! to bury the evidence. The absence of rows for a disabled worker is the
//! honest reading, and `RUN_RECORDING_AGENTS` is what stops that absence being
//! confused with "this agent never records anything".
//!
//! From the first line of the pass function onward, every path records exactly
//! one row — including the paths that decline to do any work, because "skipped:
//! Docker is not running" is the single most useful line the agent's page can
//! carry.
//!
//! The one deliberate consequence: a MANUAL pass has no `spawn` above it, so
//! each worker re-checks its own feature gate at the top of its pass body. For
//! the interval loop that re-read can never fire (its `spawn` just checked); for
//! a person pressing "run now" on a switched-off worker it is the answer they
//! asked for, and it is recorded.

use permagent::agent_runs::{self, NewRun, Trigger};
use sqlx::{Pool, Sqlite};

/// What a pass did, in the terms the run row records.
///
/// Mirrors [`permagent::agent_runs::Outcome`] but carries the data each outcome
/// needs, so a pass body can hand back "what happened" as one value instead of
/// assembling a `NewRun` at every exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassOutcome {
    /// The pass ran to completion. `examined` is the real count of units it
    /// looked at — never a fabricated zero; `None` means this pass does not
    /// count. `produced` is a one-line fact ONLY when the pass created
    /// something a reader can go and open.
    Ok {
        examined: Option<i64>,
        produced: Option<String>,
    },
    /// The pass declined to run, and says why in the worker's own words.
    Skipped { reason: String },
    /// The pass started and broke. `examined` is kept because how far it got
    /// before breaking is diagnostic.
    Failed {
        examined: Option<i64>,
        error: String,
    },
}

/// A finished pass: what to record, and what the loop must still return.
///
/// Keeping the two apart is the whole point — see the module docs. The
/// constructors set the `result` each outcome normally implies; the one place
/// a loop's historical return value differs from its honest record is spelled
/// out with [`Pass::returning`].
pub struct Pass {
    pub outcome: PassOutcome,
    pub result: Result<(), String>,
}

impl Pass {
    /// A pass that ran to completion. The loop returns `Ok(())`.
    pub fn completed(examined: Option<i64>, produced: Option<String>) -> Self {
        Self {
            outcome: PassOutcome::Ok { examined, produced },
            result: Ok(()),
        }
    }

    /// A pass that declined to run. The loop returns `Ok(())` — a skip is the
    /// worker working as designed, not an error.
    pub fn skipped(reason: impl Into<String>) -> Self {
        Self {
            outcome: PassOutcome::Skipped {
                reason: reason.into(),
            },
            result: Ok(()),
        }
    }

    /// A pass that broke. The loop returns `Err` carrying the same text the
    /// row does, so the daemon log and the agent's page cannot disagree.
    pub fn failed(examined: Option<i64>, error: impl Into<String>) -> Self {
        let error = error.into();
        Self {
            outcome: PassOutcome::Failed {
                examined,
                error: error.clone(),
            },
            result: Err(error),
        }
    }

    /// Keep the record as it is, and hand the loop the `Result` this path has
    /// always returned.
    ///
    /// Used only where the two genuinely differ, and every use is commented at
    /// its call site. This is deliberately a visible, greppable override rather
    /// than a quiet default: a run row that says "ok" because the loop returned
    /// `Ok(())` would be exactly the comfortable lie this table exists to end.
    pub fn returning(mut self, result: Result<(), String>) -> Self {
        self.result = result;
        self
    }
}

/// Write the one row for a finished pass.
///
/// Best-effort by inheritance: [`permagent::agent_runs::record`] never fails an
/// agent's real work, and it REFUSES an agent that is not declared in
/// `RUN_RECORDING_AGENTS`, so a new worker cannot start writing rows while its
/// page still claims it records none.
pub async fn record_pass(
    pool: &Pool<Sqlite>,
    agent_id: &str,
    trigger: Trigger,
    started_at: chrono::DateTime<chrono::Utc>,
    outcome: &PassOutcome,
) -> Option<String> {
    let run = match outcome {
        PassOutcome::Ok { examined, produced } => {
            NewRun::ok(agent_id, trigger, started_at, *examined, produced.clone())
        }
        PassOutcome::Skipped { reason } => {
            NewRun::skipped(agent_id, trigger, started_at, reason.clone())
        }
        PassOutcome::Failed { examined, error } => {
            NewRun::failed(agent_id, trigger, started_at, *examined, error.clone())
        }
    };
    agent_runs::record(pool, run).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use permagent::agent_runs::{recent_for_agent, Outcome};
    use permagent::session::spectral_schema;

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        spectral_schema::apply_agent_runs_schema(&pool)
            .await
            .unwrap();
        pool
    }

    fn started() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_760_000_000, 0).unwrap()
    }

    /// The row a healthy, boring sweep leaves behind. This is the whole reason
    /// the table exists: a pass that examined its projects and correctly found
    /// nothing must still be distinguishable from a pass that never happened.
    #[tokio::test]
    async fn a_pass_that_produced_nothing_still_writes_exactly_one_ok_row() {
        let pool = pool().await;
        let pass = Pass::completed(Some(3), None);
        assert!(pass.result.is_ok(), "a completed pass is not an error");

        let _ = record_pass(&pool, "strix", Trigger::Interval, started(), &pass.outcome).await;

        let runs = recent_for_agent(&pool, "strix", 10).await.unwrap();
        assert_eq!(runs.len(), 1, "one pass writes exactly one row");
        assert_eq!(runs[0].outcome, Outcome::Ok);
        assert_eq!(
            runs[0].examined,
            Some(3),
            "the real count, not a placeholder"
        );
        assert_eq!(
            runs[0].produced, None,
            "nothing was produced, so nothing is claimed"
        );
        assert_eq!(runs[0].reason, None);
    }

    /// A skip carries the worker's own words through untouched. A reason that
    /// is paraphrased on the way to the row is worse than no row at all.
    #[tokio::test]
    async fn a_skipped_pass_stores_the_reason_verbatim() {
        let pool = pool().await;
        let reason = "preflight failed: Docker is not running (`docker info` failed)";
        let _ = record_pass(
            &pool,
            "strix",
            Trigger::Interval,
            started(),
            &Pass::skipped(reason).outcome,
        )
        .await;

        let runs = recent_for_agent(&pool, "strix", 10).await.unwrap();
        assert_eq!(runs[0].outcome, Outcome::Skipped);
        assert_eq!(runs[0].reason.as_deref(), Some(reason));
        assert_eq!(
            runs[0].examined, None,
            "a pass that declined to run examined nothing — never a fabricated 0"
        );
    }

    /// How far a failing pass got is diagnostic, so `examined` survives the
    /// failure rather than being dropped on the floor.
    #[tokio::test]
    async fn a_failed_pass_keeps_both_its_error_and_how_far_it_got() {
        let pool = pool().await;
        let pass = Pass::failed(Some(1), "scan did not run: `strix` is not runnable");
        assert!(
            pass.result.is_err(),
            "a failure reaches the loop as an error"
        );

        let _ = record_pass(&pool, "strix", Trigger::Interval, started(), &pass.outcome).await;

        let runs = recent_for_agent(&pool, "strix", 10).await.unwrap();
        assert_eq!(runs[0].outcome, Outcome::Failed);
        assert_eq!(runs[0].examined, Some(1));
        assert!(runs[0].reason.as_deref().unwrap().contains("not runnable"));
    }

    /// A manual run belongs in the same history as a scheduled one, labelled
    /// for what it was — a parallel "manual runs" list would let a worker look
    /// alive on a page while its schedule has been dead for a week.
    #[tokio::test]
    async fn the_trigger_that_was_passed_is_the_trigger_that_is_stored() {
        let pool = pool().await;
        for agent in ["strix", "git_steward", "watcher"] {
            let _ = record_pass(
                &pool,
                agent,
                Trigger::Manual,
                started(),
                &Pass::completed(Some(1), None).outcome,
            )
            .await;
            let _ = record_pass(
                &pool,
                agent,
                Trigger::Interval,
                started() - chrono::Duration::seconds(60),
                &Pass::completed(Some(1), None).outcome,
            )
            .await;

            let runs = recent_for_agent(&pool, agent, 10).await.unwrap();
            assert_eq!(runs.len(), 2, "{agent} recorded both passes");
            assert_eq!(runs[0].trigger, Trigger::Manual, "{agent}: newest first");
            assert_eq!(runs[1].trigger, Trigger::Interval);
        }
    }

    /// The divergence the module docs describe, pinned: overriding the loop's
    /// `Result` must not launder the outcome. A scan that broke stays `Failed`
    /// in the record even though the sweep goes on to return `Ok(())`.
    #[test]
    fn overriding_the_loops_result_does_not_change_what_is_recorded() {
        let pass = Pass::failed(Some(1), "scan did not run: scanner exited 1").returning(Ok(()));
        assert!(pass.result.is_ok(), "the loop's behaviour is unchanged");
        assert!(
            matches!(pass.outcome, PassOutcome::Failed { .. }),
            "the record still says the pass failed"
        );

        let pass = Pass::skipped("the Guard is off (strix_enabled=false)")
            .returning(Err("the Guard is off (strix_enabled=false)".to_string()));
        assert!(
            pass.result.is_err(),
            "a manual run is told why it did nothing"
        );
        assert!(
            matches!(pass.outcome, PassOutcome::Skipped { .. }),
            "declining to run is a skip, never a failure"
        );
    }

    /// The three loops are the only callers, and every one of their ids must be
    /// declared — an undeclared id is refused by `record`, so this would fail as
    /// a silent no-row rather than as a compile error.
    #[test]
    fn every_agent_these_loops_record_for_is_declared() {
        for agent in ["strix", "git_steward", "watcher"] {
            assert!(
                agent_runs::records_runs(agent),
                "{agent} writes run rows but is not in RUN_RECORDING_AGENTS"
            );
        }
    }
}
