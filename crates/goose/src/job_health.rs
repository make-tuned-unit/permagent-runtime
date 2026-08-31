//! The truth table behind the daily job digest.
//!
//! The bug this exists to kill: the Guard attempted a scan every day from
//! 2026-08-11 to 2026-08-31, completed none, and nothing anywhere said so —
//! the failures were `tracing::warn` lines, the toggle still read "on", and
//! the Overview still showed findings from 2026-08-07. Twenty days.
//!
//! Three rules, and they are the whole design:
//!
//! 1. **Enumerate what is EXPECTED, not what was OBSERVED.** A report built
//!    from run records can never mention a job that never runs, so the bug
//!    survives it untouched. Every entry here is registered from the schedule
//!    and the enabled feature flags, and then annotated with its outcome — a
//!    job with no runs at all is a row that says `Never`, not an absence.
//! 2. **It is sent even when everything is green.** A digest that only appears
//!    when something is wrong cannot be distinguished from a digest that was
//!    never generated, which is the same class of silence it was built to end.
//!    `all_green` is a field, not a reason to skip.
//! 3. **A push is the transition; this is the standing state.** Per-event
//!    pushes fire on 0→nonzero and then go quiet (the Guard's `scan_failed`
//!    briefing, the outbox's `effect_dead_letter` briefing). Something that has
//!    been broken for a week produces no further push, so the digest is what
//!    keeps it visible — a badge or a one-time push must never be the only
//!    surface for a standing failure.
//!
//! Rendering is not this module's business: it returns the table, and the
//! notification/UI layer decides how a row looks.

use crate::scheduler::{ScheduleRunStatus, ScheduledJob};
use serde::Serialize;
use sqlx::{Pool, Row, Sqlite};

/// What a registered job's most recent outcome was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Ran, and it worked.
    Ok,
    /// Ran, and it failed.
    Failed,
    /// A window passed with the runtime down; nothing ran.
    Missed,
    /// Registered and enabled, but has never once completed.
    Never,
    /// Registered but deliberately switched off; not a failure.
    Off,
}

impl Outcome {
    /// Is this an outcome a person needs to see? `Off` is a choice, not a
    /// fault, and `Ok` is the point of the whole thing.
    pub fn is_healthy(self) -> bool {
        matches!(self, Outcome::Ok | Outcome::Off)
    }
}

/// One thing the runtime has promised to do on a cadence.
#[derive(Debug, Clone, Serialize)]
pub struct ExpectedJob {
    /// Stable id: the schedule id, or the feature id for a built-in loop.
    pub id: String,
    /// What to call it in a sentence.
    pub label: String,
    /// Human-readable cadence ("0 0 19 * * 0", "every 24h", "daily").
    pub cadence: String,
    pub outcome: Outcome,
    /// When it last ran at all (ISO-8601), success or failure.
    pub last_run: Option<String>,
    /// When it last *succeeded* (ISO-8601). The number that matters: a job can
    /// be "running daily" and not have worked in three weeks.
    pub last_success: Option<String>,
    /// Consecutive failures since the last success.
    pub failure_streak: u64,
    /// True when the last run was a startup catch-up, not an on-time fire.
    pub was_catch_up: bool,
    /// The failure in its own words, when there is one.
    pub detail: Option<String>,
}

/// The dead-letter surface. Count alone is a number, not a decision, so this
/// carries the age of the oldest entry and the reasons, grouped.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DeadLetters {
    pub dead: u64,
    pub pending: u64,
    /// ISO-8601 of the oldest dead entry, so "how long has this been true" is
    /// answerable without opening the database.
    pub oldest_dead_at: Option<String>,
    /// (reason, count), most common first.
    pub by_reason: Vec<(String, u64)>,
}

/// The whole table, as one value.
#[derive(Debug, Clone, Serialize)]
pub struct JobHealthDigest {
    pub generated_at: String,
    /// True when every expected job is healthy and no effect is dead. Reported
    /// either way — this is a field, never a reason to stay silent.
    pub all_green: bool,
    pub jobs: Vec<ExpectedJob>,
    pub dead_letters: DeadLetters,
}

impl JobHealthDigest {
    /// The rows a person actually has to act on.
    pub fn unhealthy(&self) -> Vec<&ExpectedJob> {
        self.jobs
            .iter()
            .filter(|j| !j.outcome.is_healthy())
            .collect()
    }
}

/// Feature id for the audit-chain check, which is expected work with no
/// schedule row of its own.
pub const AUDIT_CHAIN_CHECK_ID: &str = "decision_audit_chain";
/// Feature id for the Guard's sweep, likewise.
pub const GUARD_SWEEP_ID: &str = "strix";

fn outcome_from_status(status: Option<ScheduleRunStatus>) -> Outcome {
    match status {
        Some(ScheduleRunStatus::Ok) => Outcome::Ok,
        Some(ScheduleRunStatus::Error) => Outcome::Failed,
        Some(ScheduleRunStatus::Missed) => Outcome::Missed,
        // Deliberately not executed at fire time (paused when it came round).
        // A choice, not a fault.
        Some(ScheduleRunStatus::Skipped) => Outcome::Off,
        // No outcome has ever been recorded. `Never` even if `last_run` is set,
        // because a run with no recorded result is not evidence that it worked
        // — reporting it as green is the exact failure mode this module exists
        // to end.
        None => Outcome::Never,
    }
}

/// Every scheduled job, as an expected-job row. A paused job is `Off`: the
/// user turned it off, which is a state, not a failure.
pub fn scheduled_jobs(jobs: &[ScheduledJob]) -> Vec<ExpectedJob> {
    jobs.iter()
        .map(|j| {
            let outcome = if j.paused {
                Outcome::Off
            } else {
                outcome_from_status(j.last_status)
            };
            ExpectedJob {
                id: j.id.clone(),
                label: j.starter_id.clone().unwrap_or_else(|| j.id.clone()),
                cadence: if let Some(secs) = j.every_seconds {
                    format!("every {secs}s")
                } else if let Some(at) = j.at {
                    format!("once at {at}")
                } else {
                    j.cron.clone()
                },
                outcome,
                last_run: j.last_run.map(|t| t.to_rfc3339()),
                last_success: j.last_success.map(|t| t.to_rfc3339()),
                failure_streak: u64::from(j.consecutive_failures),
                was_catch_up: j.last_run_was_catch_up,
                detail: j.last_error.clone(),
            }
        })
        .collect()
}

/// The Guard's sweep, one row per project it is expected to cover. The Guard
/// is the reason this module exists, so its rows carry the distinction that
/// was missing: `last_run` is the last ATTEMPT, `last_success` is the last
/// scan that actually finished. When those two diverge for weeks, the Guard is
/// broken however green the toggle looks.
pub async fn guard_sweep_jobs(pool: &Pool<Sqlite>) -> Vec<ExpectedJob> {
    if !crate::strix::is_enabled() {
        return vec![ExpectedJob {
            id: GUARD_SWEEP_ID.to_string(),
            label: crate::strix::STRIX_NAME.to_string(),
            cadence: "daily".to_string(),
            outcome: Outcome::Off,
            last_run: None,
            last_success: None,
            failure_streak: 0,
            was_catch_up: false,
            detail: None,
        }];
    }
    let Ok(projects) = crate::projects::list_projects(pool, Some("active")).await else {
        return Vec::new();
    };
    projects
        .iter()
        .filter(|p| p.root_path.is_some() && p.id != crate::projects::PERSONAL_PROJECT_ID)
        .map(|p| {
            let meta = p.metadata_json.as_object();
            let get = |k: &str| {
                meta.and_then(|m| m.get(k))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            };
            let streak = meta
                .and_then(|m| m.get("strix_failure_streak"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let last_success = get("strix_last_scan");
            let outcome = if streak > 0 {
                Outcome::Failed
            } else if last_success.is_none() {
                Outcome::Never
            } else {
                Outcome::Ok
            };
            ExpectedJob {
                id: format!("{GUARD_SWEEP_ID}:{}", p.id),
                label: format!("{} — {}", crate::strix::STRIX_NAME, p.name),
                cadence: "daily (one project per sweep, rotating)".to_string(),
                outcome,
                last_run: get("strix_last_attempt"),
                last_success,
                failure_streak: streak,
                was_catch_up: false,
                detail: get("strix_last_error"),
            }
        })
        .collect()
}

/// The decision audit chain, verified on the digest's own cadence.
///
/// Listed as an expected check rather than left to `permagent doctor`, because
/// a check nobody runs is indistinguishable from a check that passes. It is
/// cheap enough to run daily: the walk is a single ordered scan of
/// `decision_audit` (712 rows as of 2026-08-31) recomputing one hash per row.
pub async fn audit_chain_job(pool: &Pool<Sqlite>) -> ExpectedJob {
    let now = chrono::Utc::now().to_rfc3339();
    let (outcome, detail) = match crate::decisions::verify_audit_chain(pool).await {
        Ok(report) if report.intact => (Outcome::Ok, None),
        Ok(report) => (
            Outcome::Failed,
            Some(format!(
                "the decision audit chain is broken at seq {}: {}",
                report.break_seq.unwrap_or(-1),
                report.detail
            )),
        ),
        Err(e) => (Outcome::Failed, Some(format!("could not verify: {e}"))),
    };
    ExpectedJob {
        id: AUDIT_CHAIN_CHECK_ID.to_string(),
        label: "Decision audit chain".to_string(),
        cadence: "daily".to_string(),
        outcome,
        last_run: Some(now.clone()),
        last_success: (outcome == Outcome::Ok).then_some(now),
        failure_streak: u64::from(outcome != Outcome::Ok),
        was_catch_up: false,
        detail,
    }
}

/// The dead-letter depth, oldest age and reasons.
pub async fn dead_letters(pool: &Pool<Sqlite>) -> DeadLetters {
    let dead = crate::decisions_effects::dead_effects(pool)
        .await
        .unwrap_or_default();
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM effect_outbox WHERE status IN ('pending', 'running')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let mut by_reason: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for d in &dead {
        *by_reason.entry(d.last_error.clone()).or_default() += 1;
    }
    let mut by_reason: Vec<(String, u64)> = by_reason.into_iter().collect();
    by_reason.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    DeadLetters {
        dead: dead.len() as u64,
        pending: pending.max(0) as u64,
        // `dead_effects` returns oldest first.
        oldest_dead_at: dead.first().map(|d| d.updated_at.clone()),
        by_reason,
    }
}

/// Build the whole digest. `scheduled` comes from the scheduler because the
/// daemon owns the schedule; everything else is read from the database.
pub async fn collect(pool: &Pool<Sqlite>, scheduled: &[ScheduledJob]) -> JobHealthDigest {
    let mut jobs = scheduled_jobs(scheduled);
    jobs.extend(guard_sweep_jobs(pool).await);
    jobs.push(audit_chain_job(pool).await);
    let dead_letters = dead_letters(pool).await;
    let all_green = jobs.iter().all(|j| j.outcome.is_healthy()) && dead_letters.dead == 0;
    JobHealthDigest {
        generated_at: chrono::Utc::now().to_rfc3339(),
        all_green,
        jobs,
        dead_letters,
    }
}

/// Effect rows in the outbox by status, for the `pending`/`dead` split above.
/// Exposed so a caller that already has the digest does not re-query.
pub async fn outbox_status_counts(pool: &Pool<Sqlite>) -> Vec<(String, u64)> {
    let rows = sqlx::query("SELECT status, COUNT(*) AS n FROM effect_outbox GROUP BY status")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    rows.iter()
        .map(|r| {
            let n: i64 = r.get("n");
            (r.get::<String, _>("status"), n.max(0) as u64)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str, status: Option<ScheduleRunStatus>) -> ScheduledJob {
        let now = chrono::Utc::now();
        ScheduledJob {
            id: id.to_string(),
            cron: "0 0 19 * * 0".to_string(),
            last_status: status,
            last_run: status.map(|_| now),
            last_success: (status == Some(ScheduleRunStatus::Ok)).then_some(now),
            ..Default::default()
        }
    }

    /// The rule that kills the twenty-day bug: the digest enumerates what is
    /// EXPECTED. A job that has never run once must appear as a row saying so,
    /// not be absent because there is no run to report.
    #[test]
    fn a_job_that_has_never_run_is_a_row_not_an_absence() {
        let never = job("storage-insights", None);
        let rows = scheduled_jobs(&[never]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, Outcome::Never);
        assert_eq!(rows[0].last_success, None);
        assert!(!rows[0].outcome.is_healthy());
    }

    #[test]
    fn a_missed_window_and_a_failure_are_different_rows() {
        let rows = scheduled_jobs(&[
            job("a", Some(ScheduleRunStatus::Missed)),
            job("b", Some(ScheduleRunStatus::Error)),
            job("c", Some(ScheduleRunStatus::Ok)),
        ]);
        assert_eq!(rows[0].outcome, Outcome::Missed);
        assert_eq!(rows[1].outcome, Outcome::Failed);
        assert_eq!(rows[2].outcome, Outcome::Ok);
        assert!(rows[2].last_success.is_some());
        assert!(
            rows[1].last_success.is_none(),
            "a failed run is not a success, however recently it ran"
        );
    }

    #[test]
    fn a_paused_job_is_off_not_broken() {
        let mut paused = job("workspace-snapshot", Some(ScheduleRunStatus::Missed));
        paused.paused = true;
        let rows = scheduled_jobs(&[paused]);
        assert_eq!(rows[0].outcome, Outcome::Off);
        assert!(
            rows[0].outcome.is_healthy(),
            "the user switching something off is a choice, not a fault to report"
        );
    }

    #[test]
    fn a_catch_up_run_is_marked_as_one() {
        let mut caught_up = job("storage-insights", Some(ScheduleRunStatus::Ok));
        caught_up.last_run_was_catch_up = true;
        let rows = scheduled_jobs(&[caught_up]);
        assert!(rows[0].was_catch_up);
    }

    #[tokio::test]
    async fn the_digest_reports_green_rather_than_staying_silent() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();

        let digest = collect(&pool, &[job("healthy", Some(ScheduleRunStatus::Ok))]).await;
        assert!(digest.all_green, "an empty, working system is green");
        assert!(
            digest.jobs.iter().any(|j| j.id == AUDIT_CHAIN_CHECK_ID),
            "the audit-chain check is expected work and must be listed even when it passes"
        );
        assert_eq!(digest.dead_letters.dead, 0);
        assert!(digest.unhealthy().is_empty());
        // The point: a green digest is still a digest. It has content.
        assert!(!digest.jobs.is_empty());

        let broken = collect(&pool, &[job("stale", None)]).await;
        assert!(!broken.all_green);
        assert_eq!(broken.unhealthy().len(), 1);
    }
}
