//! Durable agent run records — the answer to "did this agent actually run?".
//!
//! ## Why this table exists
//!
//! Every other durable store in this runtime records agent OUTPUT: a briefing
//! is filed when there is something to report, a card when something needs
//! approval, a finding when a scan found a vulnerability, a journal row when a
//! goal moved. All of them share one blind spot — a worker that ran, examined
//! everything it was asked to, and correctly found nothing writes NOTHING.
//! From the outside that is byte-for-byte identical to a worker that never
//! ran at all, or crashed at startup, or is silently gated off.
//!
//! That ambiguity is the whole problem this module exists to remove. A run row
//! is written on EVERY pass, including the passes that produced nothing and the
//! passes that were skipped — and a skipped pass carries the reason it was
//! skipped, because "skipped: the scanner binary is not installed" is the most
//! actionable thing this surface can say.
//!
//! ## One row per completed pass
//!
//! The row is inserted ONCE, when the pass finishes, with `started_at` and
//! `finished_at` both already known. There is deliberately no insert-at-start
//! then update-at-end: an in-flight row would have to represent "running" as a
//! nullable `finished_at`, and a daemon killed mid-pass would leave that row
//! looking permanently in-flight — a status light that lies, which is the exact
//! failure mode this surface exists to avoid. A pass that never finished simply
//! has no row, and the absence of a row for the expected interval is itself the
//! honest signal.
//!
//! ## `records_runs` is a closed list, on purpose
//!
//! [`records_runs`] names the agents whose code actually calls [`record`]. It
//! is what lets a reader distinguish two facts an empty list cannot separate:
//!
//! * "this agent records runs and has not run yet" — worth waiting for; and
//! * "this agent records no runs at all" — nothing will ever appear here, so
//!   the surface must say so rather than showing an empty list that reads as
//!   an idle agent.
//!
//! Presenting the second as the first is precisely the "looks wired, is not"
//! failure this table was added to end. `every_run_recording_agent_is_a_real_id`
//! and `record_only_accepts_declared_agents` pin the list against the roster and
//! against the write path respectively, so an agent cannot be listed here
//! without recording, nor record without being listed.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use tracing::warn;
use uuid::Uuid;

/// Run rows older than this are deleted by the startup retention pass — the
/// standing size-cap discipline the activity journal already follows.
pub const RETENTION_DAYS: i64 = 90;

/// Display caps. A run row is a summary, never a body: what a pass produced
/// lives in the briefing, card, or finding it points at.
const SUMMARY_MAX: usize = 300;
const REASON_MAX: usize = 300;

/// The agents whose code calls [`record`]. See the module docs — this list is
/// the difference between "has not run yet" and "will never appear here", and
/// a surface that cannot tell those apart is lying by omission.
///
/// Ids are worker-DESCRIPTOR ids (`git_steward`, not the `steward` persona
/// key), because that is the namespace `GET /api/agents/{id}/work` is keyed on.
/// [`crate::config::agent_identity::descriptor_id_for_worker_key`] is the
/// bridge for callers holding a persona key.
pub const RUN_RECORDING_AGENTS: &[&str] = &[
    // The Guard's project sweep (goose-server `strix::sweep_once`).
    "strix",
    // The Steward's git-health sweep (goose-server `steward_sweep::sweep_once`).
    "git_steward",
    // The Watcher's insight pass (goose-server `watcher_insights::run_once`).
    "watcher",
];

/// Whether this agent's code records runs at all.
pub fn records_runs(agent_id: &str) -> bool {
    RUN_RECORDING_AGENTS.contains(&agent_id)
}

/// What caused a pass to happen. `Manual` is the "run now" button; a row is
/// written for it exactly as for a scheduled pass, so a manual run is visible
/// in the same history rather than in a parallel one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// The worker's own interval loop.
    Interval,
    /// A person pressed "run now" on the agent's page.
    Manual,
}

impl Trigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            Trigger::Interval => "interval",
            Trigger::Manual => "manual",
        }
    }

    /// Decode the stored column. Deliberately not `FromStr`: this is a lossy
    /// column read that cannot fail. An unknown value degrades to `Interval`
    /// because that is the overwhelmingly common case, and mislabelling a
    /// stored row's trigger is cosmetic where dropping the row is not.
    pub fn from_stored(s: &str) -> Self {
        match s {
            "manual" => Trigger::Manual,
            "interval" => Trigger::Interval,
            _ => Trigger::Interval,
        }
    }
}

/// How a pass ended.
///
/// [`Outcome::Skipped`] is not a failure and must never render as one: a sweep
/// that correctly declined to run (nothing to examine, a preflight the operator
/// has not satisfied yet) is the agent working as designed, and its `reason` is
/// usually the single most useful line on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The pass ran to completion. `examined` and `produced` describe it.
    Ok,
    /// The pass declined to run. `reason` says why, and is never empty.
    Skipped,
    /// The pass started and errored. `reason` carries the error.
    Failed,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Skipped => "skipped",
            Outcome::Failed => "failed",
        }
    }

    /// Decode the stored column. Unknown values degrade to [`Outcome::Failed`]
    /// — the opposite direction from [`Trigger::from_stored`], and deliberately:
    /// an unreadable outcome must not be shown as a successful run. Failing
    /// loud on a corrupt row is the safe direction for a surface whose entire
    /// purpose is to be believed.
    pub fn from_stored(s: &str) -> Self {
        match s {
            "ok" => Outcome::Ok,
            "skipped" => Outcome::Skipped,
            "failed" => Outcome::Failed,
            _ => Outcome::Failed,
        }
    }
}

/// A completed pass, as the agent's page renders it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentRun {
    pub id: String,
    /// Worker-descriptor id. See [`RUN_RECORDING_AGENTS`] on the namespace.
    pub agent_id: String,
    pub trigger: Trigger,
    pub outcome: Outcome,
    /// RFC3339 UTC millis, so lexicographic order is chronological order.
    pub started_at: String,
    pub finished_at: String,
    /// How many units of work the pass looked at (projects, repos, memories).
    /// `None` means the pass does not count — never "zero".
    pub examined: Option<i64>,
    /// What the pass produced, in one line. `None` means it produced nothing,
    /// which for a healthy sweep is the normal case.
    pub produced: Option<String>,
    /// Why the pass was skipped or how it failed. Always set for
    /// [`Outcome::Skipped`] and [`Outcome::Failed`], always absent otherwise.
    pub reason: Option<String>,
}

/// A pass to record. Build it with [`NewRun::ok`], [`NewRun::skipped`], or
/// [`NewRun::failed`] rather than by hand — the constructors are what keep
/// `reason` present exactly when the outcome requires it.
#[derive(Debug, Clone)]
pub struct NewRun {
    pub agent_id: String,
    pub trigger: Trigger,
    pub outcome: Outcome,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub examined: Option<i64>,
    pub produced: Option<String>,
    pub reason: Option<String>,
}

impl NewRun {
    /// A pass that ran to completion. `produced` is `None` when the pass found
    /// nothing to do, which is a normal healthy result and not an error.
    pub fn ok(
        agent_id: impl Into<String>,
        trigger: Trigger,
        started_at: chrono::DateTime<chrono::Utc>,
        examined: Option<i64>,
        produced: Option<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            trigger,
            outcome: Outcome::Ok,
            started_at,
            examined,
            produced,
            reason: None,
        }
    }

    /// A pass that declined to run. The reason is required by the signature
    /// because a skip whose reason is unknown tells the reader nothing.
    pub fn skipped(
        agent_id: impl Into<String>,
        trigger: Trigger,
        started_at: chrono::DateTime<chrono::Utc>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            trigger,
            outcome: Outcome::Skipped,
            started_at,
            examined: None,
            produced: None,
            reason: Some(reason.into()),
        }
    }

    /// A pass that errored. `examined` is retained because how far a failing
    /// pass got before it broke is diagnostic.
    pub fn failed(
        agent_id: impl Into<String>,
        trigger: Trigger,
        started_at: chrono::DateTime<chrono::Utc>,
        examined: Option<i64>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            trigger,
            outcome: Outcome::Failed,
            started_at,
            examined,
            produced: None,
            reason: Some(error.into()),
        }
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let kept: String = value.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn stamp(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Record a completed pass. Best-effort by contract, exactly like
/// [`crate::briefings::file_briefing`]: an agent's real work must never fail
/// because the record of it could not be written. Returns the row id so a
/// caller can correlate its own logs.
///
/// A run for an agent not in [`RUN_RECORDING_AGENTS`] is REFUSED rather than
/// written. That is not defensiveness about bad input — it is the pin that
/// keeps the list honest: an agent cannot start recording runs without being
/// declared, so the surface's "records no runs" sentence can never be stale.
pub async fn record(pool: &Pool<Sqlite>, run: NewRun) -> Option<String> {
    if !records_runs(&run.agent_id) {
        warn!(
            target: "permagent::agent_runs",
            "refusing a run row for '{}': not in RUN_RECORDING_AGENTS, so the agent's page \
             would still say it records no runs. Add it to that list in the same change.",
            run.agent_id
        );
        return None;
    }
    let id = Uuid::now_v7().to_string();
    let finished_at = stamp(chrono::Utc::now());
    let result = sqlx::query(
        "INSERT INTO agent_runs
            (id, agent_id, trigger, outcome, started_at, finished_at, examined, produced, reason)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&run.agent_id)
    .bind(run.trigger.as_str())
    .bind(run.outcome.as_str())
    .bind(stamp(run.started_at))
    .bind(&finished_at)
    .bind(run.examined)
    .bind(run.produced.as_deref().map(|v| truncate(v, SUMMARY_MAX)))
    .bind(run.reason.as_deref().map(|v| truncate(v, REASON_MAX)))
    .execute(pool)
    .await;

    match result {
        Ok(_) => Some(id),
        Err(e) => {
            warn!(
                target: "permagent::agent_runs",
                "failed to record a run for {}: {}", run.agent_id, e
            );
            None
        }
    }
}

fn row_to_run(row: &sqlx::sqlite::SqliteRow) -> AgentRun {
    AgentRun {
        id: row.get("id"),
        agent_id: row.get("agent_id"),
        trigger: Trigger::from_stored(row.get::<String, _>("trigger").as_str()),
        outcome: Outcome::from_stored(row.get::<String, _>("outcome").as_str()),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        examined: row.get("examined"),
        produced: row.get("produced"),
        reason: row.get("reason"),
    }
}

/// This agent's most recent passes, newest first.
///
/// Errors propagate rather than degrading to an empty list: on a surface whose
/// job is to distinguish "nothing happened" from "we could not look", an
/// unreadable table rendering as an empty history would be the same lie the
/// whole feature exists to remove.
pub async fn recent_for_agent(
    pool: &Pool<Sqlite>,
    agent_id: &str,
    limit: i64,
) -> Result<Vec<AgentRun>> {
    let rows = sqlx::query(
        "SELECT id, agent_id, trigger, outcome, started_at, finished_at, examined, produced, reason
         FROM agent_runs WHERE agent_id = ? ORDER BY started_at DESC LIMIT ?",
    )
    .bind(agent_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_run).collect())
}

/// Drop run rows past the retention window. Called from the same startup pass
/// that trims the activity journal.
pub async fn prune(pool: &Pool<Sqlite>) -> Result<u64> {
    let cutoff = stamp(chrono::Utc::now() - chrono::Duration::days(RETENTION_DAYS));
    let result = sqlx::query("DELETE FROM agent_runs WHERE started_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema;

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        spectral_schema::apply_agent_runs_schema(&pool)
            .await
            .unwrap();
        pool
    }

    fn t(offset_secs: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_760_000_000 + offset_secs, 0).unwrap()
    }

    /// The list is the surface's honesty contract, so it may not name an agent
    /// the roster does not have — a typo here would make the page claim an
    /// agent records runs while nothing ever writes one.
    #[test]
    fn every_run_recording_agent_is_a_real_id() {
        for id in RUN_RECORDING_AGENTS {
            assert!(
                crate::agents::self_knowledge::WORKER_DESCRIPTORS
                    .iter()
                    .any(|d| &d.id == id),
                "'{id}' is in RUN_RECORDING_AGENTS but is not a worker-descriptor id"
            );
        }
    }

    /// The other half of the same pin. Writing without being declared would
    /// leave real rows in the table under an agent whose page says it records
    /// none — the surface would contradict its own data.
    #[tokio::test]
    async fn record_only_accepts_declared_agents() {
        let pool = pool().await;
        assert!(
            record(
                &pool,
                NewRun::ok("librarian", Trigger::Interval, t(0), None, None)
            )
            .await
            .is_none(),
            "an undeclared agent must not be able to write a run row"
        );
        assert!(recent_for_agent(&pool, "librarian", 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_pass_that_produced_nothing_is_still_recorded() {
        let pool = pool().await;
        record(
            &pool,
            NewRun::ok("strix", Trigger::Interval, t(0), Some(4), None),
        )
        .await
        .expect("a clean pass must be recorded");
        let runs = recent_for_agent(&pool, "strix", 10).await.unwrap();
        assert_eq!(
            runs.len(),
            1,
            "a pass that found nothing is still evidence it ran"
        );
        assert_eq!(runs[0].outcome, Outcome::Ok);
        assert_eq!(runs[0].examined, Some(4));
        assert_eq!(runs[0].produced, None);
        assert_eq!(runs[0].reason, None);
    }

    #[tokio::test]
    async fn a_skip_carries_its_reason() {
        let pool = pool().await;
        record(
            &pool,
            NewRun::skipped(
                "strix",
                Trigger::Interval,
                t(0),
                "the scanner is not installed",
            ),
        )
        .await
        .unwrap();
        let runs = recent_for_agent(&pool, "strix", 10).await.unwrap();
        assert_eq!(runs[0].outcome, Outcome::Skipped);
        assert_eq!(
            runs[0].reason.as_deref(),
            Some("the scanner is not installed")
        );
    }

    #[tokio::test]
    async fn runs_come_back_newest_first_and_scoped_to_the_agent() {
        let pool = pool().await;
        record(
            &pool,
            NewRun::ok("strix", Trigger::Interval, t(0), Some(1), None),
        )
        .await
        .unwrap();
        record(
            &pool,
            NewRun::ok("strix", Trigger::Manual, t(60), Some(2), None),
        )
        .await
        .unwrap();
        record(
            &pool,
            NewRun::ok("watcher", Trigger::Interval, t(30), Some(9), None),
        )
        .await
        .unwrap();

        let strix = recent_for_agent(&pool, "strix", 10).await.unwrap();
        assert_eq!(strix.len(), 2, "another agent's runs must not leak in");
        assert_eq!(strix[0].trigger, Trigger::Manual, "newest first");
        assert_eq!(strix[1].trigger, Trigger::Interval);

        let watcher = recent_for_agent(&pool, "watcher", 10).await.unwrap();
        assert_eq!(watcher.len(), 1);
        assert_eq!(watcher[0].examined, Some(9));
    }

    /// An unreadable table must surface as an error, never as "no runs" — the
    /// two mean opposite things to someone asking whether their agent works.
    #[tokio::test]
    async fn a_missing_table_is_an_error_not_an_empty_history() {
        let bare = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        assert!(recent_for_agent(&bare, "strix", 10).await.is_err());
    }

    #[tokio::test]
    async fn prune_drops_only_rows_past_the_window() {
        let pool = pool().await;
        let old = chrono::Utc::now() - chrono::Duration::days(RETENTION_DAYS + 1);
        let recent = chrono::Utc::now() - chrono::Duration::days(1);
        record(
            &pool,
            NewRun::ok("strix", Trigger::Interval, old, None, None),
        )
        .await
        .unwrap();
        record(
            &pool,
            NewRun::ok("strix", Trigger::Interval, recent, None, None),
        )
        .await
        .unwrap();
        assert_eq!(prune(&pool).await.unwrap(), 1);
        let left = recent_for_agent(&pool, "strix", 10).await.unwrap();
        assert_eq!(left.len(), 1);
    }

    /// A corrupt outcome must not read as a successful run. This is the one
    /// decode in the module that degrades toward the alarming value.
    #[test]
    fn an_unreadable_outcome_degrades_to_failed_not_ok() {
        assert_eq!(Outcome::from_stored("ok"), Outcome::Ok);
        assert_eq!(Outcome::from_stored("skipped"), Outcome::Skipped);
        assert_eq!(Outcome::from_stored("failed"), Outcome::Failed);
        assert_eq!(Outcome::from_stored("nonsense"), Outcome::Failed);
    }

    #[test]
    fn long_text_is_capped_rather_than_rejected() {
        let long = "x".repeat(SUMMARY_MAX * 2);
        let capped = truncate(&long, SUMMARY_MAX);
        assert_eq!(capped.chars().count(), SUMMARY_MAX);
        assert!(capped.ends_with('…'));
    }
}
