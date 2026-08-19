//! The measurement pass: for every verified action, judge each whole-week
//! window whose last day is over.
//!
//! Idempotent by construction — it recomputes a window from the events table and
//! `INSERT OR REPLACE`s the verdict, so running it twice in a night, or after a
//! late drain backfilled events, converges rather than duplicating.

use super::metrics::{
    self, DateRange, MetricWindow, TargetDir, TargetMetric, HISTORY_WEEKS, WINDOW_DAYS,
};
use super::power::{self, JudgeInput, Judgement};
use super::store::{self, GrowthActionRow, STATUS_ARCHIVED, STATUS_JUDGED, STATUS_MEASURING};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

/// The pre-registration snapshot written at verify time.
///
/// The `before` windows are computed and stored HERE rather than at measurement
/// time, because by the time the 28-day window closes the events they were
/// counted from may be gone: `analytics_drain` prunes at 400 days
/// (analytics_drain.rs:38), and the whole before window is already in the past
/// at verify time anyway. Freezing it also means a verdict cannot quietly change
/// its own baseline between the 7-day and the 28-day reading.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Baseline {
    pub metric: TargetMetric,
    pub dir: TargetDir,
    /// First fully-post-change UTC day, `YYYY-MM-DD`.
    pub pivot: String,
    pub taken_at: String,
    /// `window_days` → the matching before window.
    pub before: std::collections::BTreeMap<u32, MetricWindow>,
    /// Whole-week totals of the metric leading up to the action, oldest first,
    /// zero-filled. This is what the power check's variance estimate reads.
    pub weekly: Vec<f64>,
    /// Earliest event date the database holds for this project, or `None` when
    /// it holds none.
    pub earliest_event: Option<String>,
}

/// Compute and freeze the baseline for a newly verified action.
pub async fn snapshot_baseline(
    pool: &Pool<Sqlite>,
    project_id: &str,
    metric: TargetMetric,
    dir: TargetDir,
    verified_at: DateTime<Utc>,
) -> anyhow::Result<Baseline> {
    let pivot = metrics::pivot_date(verified_at);
    let mut before = std::collections::BTreeMap::new();
    for days in WINDOW_DAYS {
        let (b, _) = metrics::windows_for(pivot, days);
        before.insert(
            days,
            metrics::load_window(pool, project_id, metric, b).await?,
        );
    }
    // History ends where the shortest before-window begins, so the variance
    // estimate and the comparison never read the same weeks — reusing them would
    // measure the delta against a yardstick built from itself.
    let history_end = pivot - Duration::days(1 + i64::from(WINDOW_DAYS[0]));
    let weekly =
        metrics::weekly_history(pool, project_id, metric, history_end, HISTORY_WEEKS).await?;

    Ok(Baseline {
        metric,
        dir,
        pivot: pivot.format("%Y-%m-%d").to_string(),
        taken_at: Utc::now().to_rfc3339(),
        before,
        weekly,
        earliest_event: metrics::earliest_event_date(pool, project_id).await?,
    })
}

/// Why a window produced no row this pass. Not an error — most actions are
/// simply waiting for the calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skipped {
    /// The window's last day has not happened yet.
    NotDue,
    /// No baseline was frozen for this window at verify time.
    NoBaseline,
}

#[derive(Debug, Clone)]
pub struct WindowResult {
    pub window_days: u32,
    pub judgement: Judgement,
}

/// Does the before window reach back past what the database ever recorded?
///
/// A window that starts before the first event is not a quiet period; it is a
/// period nothing was watching, either because the relay was not installed or
/// because retention removed it. The counts look identical, so the only honest
/// move is to refuse.
fn coverage_gap(before: &MetricWindow, earliest_event: Option<&str>, days: u32) -> Option<String> {
    let earliest = earliest_event?;
    if before.start.as_str() >= earliest {
        return None;
    }
    Some(format!(
        "The {days} days before this action start on {}, but the earliest traffic this project \
         ever recorded is {earliest}, so part of the baseline is a period nothing was measuring \
         rather than a quiet one.",
        before.start
    ))
}

/// Judge one window, or say why it was skipped.
pub async fn measure_window(
    pool: &Pool<Sqlite>,
    action: &GrowthActionRow,
    baseline: &Baseline,
    days: u32,
    now: DateTime<Utc>,
) -> anyhow::Result<Result<WindowResult, Skipped>> {
    let pivot = NaiveDate::parse_from_str(&baseline.pivot, "%Y-%m-%d")?;
    if !metrics::window_is_complete(pivot, days, now) {
        return Ok(Err(Skipped::NotDue));
    }
    let Some(before) = baseline.before.get(&days) else {
        return Ok(Err(Skipped::NoBaseline));
    };

    let (_, after_range) = metrics::windows_for(pivot, days);
    let after =
        metrics::load_window(pool, &action.project_id, baseline.metric, after_range).await?;

    // Confounders are looked for across the WHOLE comparison span, before window
    // included: a change verified three days before this one is still sitting
    // inside the "before" numbers this verdict rests on.
    let span = DateRange {
        start: before.start.parse()?,
        end: after_range.end,
    };
    let confounders = store::verified_between(
        pool,
        &action.project_id,
        &span.start.format("%Y-%m-%d").to_string(),
        &span.end.format("%Y-%m-%d").to_string(),
        &action.id,
    )
    .await?;

    let judgement = power::judge(JudgeInput {
        dir: baseline.dir,
        before,
        after: &after,
        weekly: &baseline.weekly,
        confounders: &confounders,
        coverage_gap: coverage_gap(before, baseline.earliest_event.as_deref(), days),
    });

    store::upsert_outcome(
        pool,
        &action.id,
        days,
        before,
        &after,
        &judgement,
        &now.to_rfc3339(),
    )
    .await?;

    Ok(Ok(WindowResult {
        window_days: days,
        judgement,
    }))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub actions_considered: usize,
    pub windows_judged: usize,
    pub actions_completed: usize,
    pub errors: Vec<String>,
}

/// Judge every due window for every verified, pre-registered action.
///
/// Errors on one action are collected, not propagated: a project whose analytics
/// table is in a strange state must not stop the other eighteen from being
/// measured.
pub async fn run(pool: &Pool<Sqlite>, now: DateTime<Utc>) -> anyhow::Result<SweepReport> {
    let mut report = SweepReport::default();
    for action in store::pending_measurement(pool).await? {
        report.actions_considered += 1;
        let Some(raw) = action.baseline_json.as_deref() else {
            continue;
        };
        let baseline: Baseline = match serde_json::from_str(raw) {
            Ok(b) => b,
            Err(e) => {
                report
                    .errors
                    .push(format!("action {}: unreadable baseline: {e}", action.id));
                continue;
            }
        };

        let mut judged = 0usize;
        for days in WINDOW_DAYS {
            match measure_window(pool, &action, &baseline, days, now).await {
                Ok(Ok(_)) => {
                    judged += 1;
                    report.windows_judged += 1;
                }
                Ok(Err(_)) => {}
                Err(e) => report
                    .errors
                    .push(format!("action {} at {days}d: {e}", action.id)),
            }
        }

        // `judged` while any window is still open, `judged` only once the
        // longest one has been read — an action that stops at 7 days would
        // freeze the provisional reading as the final word.
        let all_done = judged == WINDOW_DAYS.len();
        let next = if all_done {
            STATUS_JUDGED
        } else {
            STATUS_MEASURING
        };
        // An archived action keeps being MEASURED (see
        // `store::pending_measurement`) but never moves status. Archiving is the
        // user filing a card away; setting it back to `measuring` or `judged`
        // would push it onto the board they just cleared, and they would have
        // to archive it again after every pass. The outcomes are still written,
        // so the verdict lands — the card simply stays filed.
        if action.status != next && judged > 0 && action.status != STATUS_ARCHIVED {
            if let Err(e) =
                store::set_status(pool, &action.project_id, &action.id, next, None).await
            {
                report.errors.push(format!("action {}: {e}", action.id));
            }
        }
        if all_done {
            report.actions_completed += 1;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::growth::power::Verdict;
    use crate::growth::store::{ActionSeed, VERIFIED_BY_GIT};
    use crate::session::spectral_schema::{
        apply_analytics_events_schema, apply_growth_actions_schema,
    };
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        apply_analytics_events_schema(&pool).await.unwrap();
        apply_growth_actions_schema(&pool).await.unwrap();
        pool
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// Insert `views` pageviews on `date`, each its own session so `sessions`
    /// and `pageviews` are both exercised. One statement per day: these tests
    /// insert five figures of rows and a statement per row makes them slow
    /// enough to skip, which is how coverage rots.
    async fn traffic(pool: &Pool<Sqlite>, project: &str, date: NaiveDate, views: u32) {
        if views == 0 {
            return;
        }
        let values: Vec<String> = (0..views)
            .map(|i| {
                format!(
                    "('{project}', 'pageview', '/', 'v{i}', '{date}-s{i}', 0, '{date}T12:00:00Z')"
                )
            })
            .collect();
        sqlx::query(&format!(
            "INSERT INTO analytics_events
                (project_id, kind, path, visitor_hash, session_id, is_bot, created_at)
             VALUES {}",
            values.join(",")
        ))
        .execute(pool)
        .await
        .unwrap();
    }

    /// Fill `[start, end)` with `per_day` views a day.
    async fn traffic_over(
        pool: &Pool<Sqlite>,
        project: &str,
        start: NaiveDate,
        days: i64,
        per_day: u32,
    ) {
        for i in 0..days {
            traffic(pool, project, start + Duration::days(i), per_day).await;
        }
    }

    async fn verified_action(
        pool: &Pool<Sqlite>,
        project: &str,
        title: &str,
        verified_at: &str,
        metric: TargetMetric,
        dir: TargetDir,
    ) -> (GrowthActionRow, Baseline) {
        let row = store::upsert_suggested(
            pool,
            project,
            &ActionSeed {
                title: title.into(),
                recommendation: format!("recommendation for {title}"),
                category: Some("seo".into()),
                artifact_kind: Some("prompt".into()),
                artifact: None,
                target_metric: None,
                target_dir: None,
            },
        )
        .await
        .unwrap();
        store::set_status(
            pool,
            project,
            &row.id,
            store::STATUS_DONE,
            Some((metric, dir)),
        )
        .await
        .unwrap();
        let baseline = snapshot_baseline(pool, project, metric, dir, at(verified_at))
            .await
            .unwrap();
        let row = store::record_verification(
            pool,
            project,
            &row.id,
            VERIFIED_BY_GIT,
            verified_at,
            Some(&serde_json::to_string(&baseline).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        (row, baseline)
    }

    /// End to end over real event rows: a genuine, large lift on traffic that
    /// can carry a verdict must come out as `helped`, with the numbers in the
    /// rationale.
    #[tokio::test]
    async fn a_real_lift_on_real_rows_is_judged_helped() {
        let pool = pool().await;
        // 100 views/day for the 13 weeks before, 160/day after.
        traffic_over(&pool, "p1", day("2026-05-01"), 103, 100).await;
        traffic_over(&pool, "p1", day("2026-08-12"), 28, 160).await;

        let (action, baseline) = verified_action(
            &pool,
            "p1",
            "Add FAQ schema",
            "2026-08-11T14:00:00Z",
            TargetMetric::Pageviews,
            TargetDir::Up,
        )
        .await;
        assert_eq!(baseline.pivot, "2026-08-12");
        assert_eq!(baseline.before[&7].value, 700.0);
        assert_eq!(baseline.weekly.len(), 12);

        let out = measure_window(&pool, &action, &baseline, 7, at("2026-08-20T03:00:00Z"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            out.judgement.verdict,
            Verdict::Helped,
            "{:?}",
            out.judgement
        );
        assert!(
            out.judgement.rationale.contains("700 to 1120"),
            "{}",
            out.judgement.rationale
        );

        let stored = store::outcomes_for(&pool, &action.id).await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].verdict, "helped");
        assert!(!stored[0].rationale.is_empty());
    }

    /// The same code path over a flat series must not manufacture a verdict.
    #[tokio::test]
    async fn a_flat_project_is_never_judged_helped() {
        let pool = pool().await;
        traffic_over(&pool, "p1", day("2026-05-01"), 131, 100).await;

        let (action, baseline) = verified_action(
            &pool,
            "p1",
            "Rewrite hero copy",
            "2026-08-11T14:00:00Z",
            TargetMetric::Pageviews,
            TargetDir::Up,
        )
        .await;
        let out = measure_window(&pool, &action, &baseline, 7, at("2026-08-20T03:00:00Z"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            out.judgement.verdict,
            Verdict::NoEffect,
            "{}",
            out.judgement.rationale
        );
        assert_eq!(out.judgement.delta_pct, Some(0.0));
    }

    /// A partial week is a different mix of weekdays than the full week it
    /// would be compared against, so nothing is said until it closes.
    #[tokio::test]
    async fn a_window_that_has_not_closed_yet_is_not_judged() {
        let pool = pool().await;
        traffic_over(&pool, "p1", day("2026-05-01"), 110, 100).await;
        let (action, baseline) = verified_action(
            &pool,
            "p1",
            "t",
            "2026-08-11T14:00:00Z",
            TargetMetric::Pageviews,
            TargetDir::Up,
        )
        .await;

        // 2026-08-18 is the last day of the 7-day window; nothing before the
        // 19th.
        assert!(matches!(
            measure_window(&pool, &action, &baseline, 7, at("2026-08-18T23:00:00Z"))
                .await
                .unwrap(),
            Err(Skipped::NotDue)
        ));
        assert!(store::outcomes_for(&pool, &action.id)
            .await
            .unwrap()
            .is_empty());
        assert!(
            measure_window(&pool, &action, &baseline, 28, at("2026-09-30T00:00:00Z"))
                .await
                .unwrap()
                .is_ok()
        );
    }

    /// The confounding rule, over real rows: a second action verified inside the
    /// comparison span makes both unattributable.
    #[tokio::test]
    async fn a_second_change_inside_the_window_confounds_it() {
        let pool = pool().await;
        traffic_over(&pool, "p1", day("2026-05-01"), 103, 100).await;
        traffic_over(&pool, "p1", day("2026-08-12"), 28, 160).await;

        let (action, baseline) = verified_action(
            &pool,
            "p1",
            "Add FAQ schema",
            "2026-08-11T14:00:00Z",
            TargetMetric::Pageviews,
            TargetDir::Up,
        )
        .await;
        verified_action(
            &pool,
            "p1",
            "Rewrite hero copy",
            "2026-08-14T10:00:00Z",
            TargetMetric::Pageviews,
            TargetDir::Up,
        )
        .await;

        let out = measure_window(&pool, &action, &baseline, 7, at("2026-08-20T03:00:00Z"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(out.judgement.verdict, Verdict::Confounded);
        assert!(
            out.judgement.rationale.contains("\"Rewrite hero copy\""),
            "{}",
            out.judgement.rationale
        );
        let stored = store::outcomes_for(&pool, &action.id).await.unwrap();
        assert!(stored[0]
            .confounders
            .as_deref()
            .unwrap()
            .contains("Rewrite"));
    }

    /// A baseline reaching back before the first event this project ever
    /// recorded is a period nothing was watching, not a quiet one.
    #[tokio::test]
    async fn a_baseline_older_than_the_data_is_refused() {
        let pool = pool().await;
        // Traffic starts only three days before the action.
        traffic_over(&pool, "p1", day("2026-08-08"), 4, 100).await;
        traffic_over(&pool, "p1", day("2026-08-12"), 8, 400).await;

        let (action, baseline) = verified_action(
            &pool,
            "p1",
            "t",
            "2026-08-11T14:00:00Z",
            TargetMetric::Pageviews,
            TargetDir::Up,
        )
        .await;
        assert_eq!(baseline.earliest_event.as_deref(), Some("2026-08-08"));

        let out = measure_window(&pool, &action, &baseline, 7, at("2026-08-20T03:00:00Z"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(out.judgement.verdict, Verdict::Inconclusive);
        assert!(
            out.judgement.rationale.contains("nothing was measuring"),
            "{}",
            out.judgement.rationale
        );
    }

    /// The sweep judges what is due, leaves what is not, and only calls an
    /// action finished once its longest window has been read.
    #[tokio::test]
    async fn the_sweep_judges_due_windows_and_finishes_at_the_longest() {
        let pool = pool().await;
        traffic_over(&pool, "p1", day("2026-05-01"), 103, 100).await;
        traffic_over(&pool, "p1", day("2026-08-12"), 28, 160).await;
        let (action, _) = verified_action(
            &pool,
            "p1",
            "t",
            "2026-08-11T14:00:00Z",
            TargetMetric::Pageviews,
            TargetDir::Up,
        )
        .await;

        // Only the 7-day window has closed.
        let mid = run(&pool, at("2026-08-20T03:00:00Z")).await.unwrap();
        assert_eq!(mid.actions_considered, 1);
        assert_eq!(mid.windows_judged, 1);
        assert_eq!(mid.actions_completed, 0);
        assert!(mid.errors.is_empty(), "{:?}", mid.errors);
        assert_eq!(
            store::get(&pool, "p1", &action.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            STATUS_MEASURING
        );

        // All three closed. This pass re-judges the 7-day window it already
        // judged above, and there are still exactly three rows — the
        // INSERT OR REPLACE against PRIMARY KEY(action_id, window_days) is what
        // keeps a nightly re-evaluation from stacking verdicts.
        let done = run(&pool, at("2026-09-10T03:00:00Z")).await.unwrap();
        assert_eq!(done.windows_judged, 3);
        assert_eq!(done.actions_completed, 1);
        let outcomes = store::outcomes_for(&pool, &action.id).await.unwrap();
        assert_eq!(
            outcomes.iter().map(|o| o.window_days).collect::<Vec<_>>(),
            vec![7, 14, 28]
        );
        assert_eq!(
            store::get(&pool, "p1", &action.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            STATUS_JUDGED
        );

        // A finished action drops out of the sweep entirely rather than being
        // re-read every night forever.
        let again = run(&pool, at("2026-09-10T04:00:00Z")).await.unwrap();
        assert_eq!(again.actions_considered, 0);
        assert_eq!(
            store::outcomes_for(&pool, &action.id).await.unwrap().len(),
            3
        );
    }

    /// REGRESSION. `pending_measurement` was widened so an archived action
    /// still owed a window keeps being measured; on its own that would let this
    /// same loop set the archived action's status back to `measuring` and then
    /// `judged`, putting the card the user just filed away back on the board on
    /// the next tick. The two changes only make sense together, so this test
    /// asserts both halves: the outcomes are written AND the status is
    /// untouched.
    #[tokio::test]
    async fn the_sweep_does_not_pull_an_archived_action_back_onto_the_board() {
        let pool = pool().await;
        traffic_over(&pool, "p1", day("2026-05-01"), 103, 100).await;
        traffic_over(&pool, "p1", day("2026-08-12"), 28, 160).await;
        let (action, _) = verified_action(
            &pool,
            "p1",
            "t",
            "2026-08-11T14:00:00Z",
            TargetMetric::Pageviews,
            TargetDir::Up,
        )
        .await;
        store::set_status(&pool, "p1", &action.id, STATUS_ARCHIVED, None)
            .await
            .unwrap();

        let report = run(&pool, at("2026-09-10T03:00:00Z")).await.unwrap();
        assert_eq!(report.windows_judged, 3, "the verdict still lands");
        assert_eq!(report.actions_completed, 1);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(
            store::outcomes_for(&pool, &action.id).await.unwrap().len(),
            3
        );
        assert_eq!(
            store::get(&pool, "p1", &action.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            STATUS_ARCHIVED,
            "a filed card must not be dragged back to measuring or judged"
        );
        // And with every window written it drops out of the sweep for good.
        let again = run(&pool, at("2026-09-10T04:00:00Z")).await.unwrap();
        assert_eq!(again.actions_considered, 0);
    }

    /// The history that feeds the variance estimate must not overlap the
    /// windows being compared, or the delta is measured against a yardstick
    /// built partly from itself.
    #[tokio::test]
    async fn the_variance_history_stops_before_the_before_window() {
        let pool = pool().await;
        traffic_over(&pool, "p1", day("2026-05-01"), 103, 100).await;
        let baseline = snapshot_baseline(
            &pool,
            "p1",
            TargetMetric::Pageviews,
            TargetDir::Up,
            at("2026-08-11T14:00:00Z"),
        )
        .await
        .unwrap();
        // pivot 2026-08-12, shortest before window [2026-08-04, 2026-08-11);
        // history therefore ends 2026-08-04 and covers the 12 weeks before it.
        assert_eq!(baseline.before[&7].start, "2026-08-04");
        assert_eq!(baseline.weekly.len(), 12);
        assert!(
            baseline.weekly.iter().all(|w| (*w - 700.0).abs() < 1e-9),
            "{:?}",
            baseline.weekly
        );
    }
}
