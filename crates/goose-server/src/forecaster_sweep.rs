//! The Forecaster's collection pass.
//!
//! Reads approved series out of the registry, asks each one's source for the
//! window it does not already have, and appends what comes back. It never
//! approves a series, never invents a subject, and never calls a model — this
//! pass is HTTP and arithmetic, in the same spirit as `growth_sweep`.
//!
//! **Cadence is a knob, and its default is weekly.** Direction does not move on
//! a one-day scale, and weekly halves the row count. The loop still ticks
//! hourly so that a machine asleep on Sunday night collects on Monday rather
//! than skipping the week — the schedule is "at least this often", not "only
//! then". Sunday after 22:00 local is the *preferred* window, which is what
//! pulls a fresh install onto Sunday night and keeps it there.
//!
//! **A dead collector is recorded, not swallowed.** `store::mark_collected`
//! stores the error on the series, because a collector that silently stopped
//! reads exactly like a market that stopped moving, and those are different
//! answers.

use crate::state::AppState;
use chrono::{Datelike, Timelike};
use permagent::forecaster::collect::{self, Fetcher};
use permagent::forecaster::store;
use permagent::forecaster::{CollectionCadence, Knobs, SourceKind};
use std::sync::Arc;
use std::time::Duration;

/// Let boot settle. Nothing here is due more often than once a day, and the
/// first pass of a fresh install may make hundreds of backfill requests.
const STARTUP_DELAY: Duration = Duration::from_secs(600);

/// Check hourly, collect when due. The tick is not the cadence.
const TICK: Duration = Duration::from_secs(3600);

/// The window a weekly pass prefers to land in: Sunday, 22:00 local onward.
const PREFERRED_WEEKDAY: chrono::Weekday = chrono::Weekday::Sun;
const PREFERRED_HOUR: u32 = 22;

pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            run_once(&state).await;
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn run_once(state: &AppState) {
    let pool = match state.session_manager().pool_clone().await {
        Ok(pool) => pool,
        Err(e) => {
            tracing::debug!(target: "permagentd::forecaster", "collection pass skipped: {e}");
            return;
        }
    };
    let knobs = Knobs::load();
    let fetcher = match collect::HttpFetcher::new() {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(target: "permagentd::forecaster", "no HTTP client: {e}");
            return;
        }
    };
    let report = collect_due(&pool, &fetcher, &knobs, chrono::Local::now()).await;
    if report.collected == 0 && report.errors.is_empty() {
        // Most passes find nothing due. A log line per tick would bury the
        // ones that actually collected something.
        return;
    }
    tracing::info!(
        target: "permagentd::forecaster",
        considered = report.considered,
        collected = report.collected,
        points = report.points_added,
        "market collection pass"
    );
    for error in &report.errors {
        tracing::warn!(target: "permagentd::forecaster", "collection error: {error}");
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct CollectionReport {
    pub considered: usize,
    pub collected: usize,
    pub points_added: usize,
    pub skipped_not_due: usize,
    pub errors: Vec<String>,
}

/// Is this series due, given the knob and the clock?
///
/// Split out and pure so the schedule is testable without waiting a week.
pub fn is_due(
    cadence: CollectionCadence,
    last_collected_at: Option<&str>,
    now: chrono::DateTime<chrono::Local>,
) -> bool {
    let Some(last) = last_collected_at
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&chrono::Local))
    else {
        // Never collected: due now. A bind that waits a week to show its
        // backfill looks broken.
        return true;
    };
    let elapsed = now - last;
    match cadence {
        CollectionCadence::Daily => elapsed >= chrono::Duration::hours(20),
        CollectionCadence::Weekly => {
            if elapsed >= chrono::Duration::days(7) {
                return true;
            }
            // Pull the schedule onto Sunday night and hold it there, without
            // ever letting a series go more than eight days stale.
            elapsed >= chrono::Duration::days(6)
                && now.weekday() == PREFERRED_WEEKDAY
                && now.hour() >= PREFERRED_HOUR
        }
    }
}

/// Collect every active series that is due. Exposed for tests, which drive it
/// with a fixture fetcher and a fixed clock.
pub async fn collect_due(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    fetcher: &dyn Fetcher,
    knobs: &Knobs,
    now: chrono::DateTime<chrono::Local>,
) -> CollectionReport {
    let mut report = CollectionReport::default();
    let series = match store::active_series(pool).await {
        Ok(s) => s,
        Err(e) => {
            report.errors.push(format!("read registry: {e}"));
            return report;
        }
    };
    report.considered = series.len();
    let today = now.date_naive();
    for s in series {
        if !is_due(knobs.cadence, s.last_collected_at.as_deref(), now) {
            report.skipped_not_due += 1;
            continue;
        }
        // Equity closes stay a read-time dependency unless the user says
        // otherwise: `market_data.rs` already documents that endpoint as
        // unsupported, and writing months of it here would make it durable.
        if s.source_kind == SourceKind::EquityClose && !knobs.persist_equity_closes {
            report.skipped_not_due += 1;
            continue;
        }
        let last_ts = match store::load_points(pool, &s.id).await {
            Ok(points) => points.last().map(|p| p.ts.clone()),
            Err(e) => {
                report.errors.push(format!("{}: {e}", s.label));
                continue;
            }
        };
        let (since, until) = collect::incremental_window(last_ts.as_deref(), today);
        let outcome =
            collect::collect(fetcher, s.source_kind, &s.subject, s.cadence, since, until).await;
        match outcome {
            Ok(got) => match store::append_points(pool, &s.id, &got.points).await {
                Ok(n) => {
                    report.collected += 1;
                    report.points_added += n;
                    let _ = store::mark_collected(pool, &s.id, None).await;
                }
                Err(e) => {
                    report.errors.push(format!("{}: {e}", s.label));
                    let _ = store::mark_collected(pool, &s.id, Some(&e)).await;
                }
            },
            Err(e) => {
                let msg = e.to_string();
                report.errors.push(format!("{}: {msg}", s.label));
                // Recorded on the row, so `forecaster_series` and the Market
                // card can say "collector stale" with the reason attached.
                let _ = store::mark_collected(pool, &s.id, Some(&msg)).await;
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use permagent::forecaster::collect::FixtureFetcher;
    use permagent::forecaster::series::SeriesStatus;
    use permagent::forecaster::Cadence;
    use sqlx::{Pool, Sqlite};

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        permagent::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        pool
    }

    fn at(y: i32, m: u32, d: u32, h: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    const NPM_BODY: &str = r#"{"package":"vitest","downloads":[
        {"downloads":1200,"day":"2026-08-22"},{"downloads":1310,"day":"2026-08-23"}]}"#;

    #[test]
    fn a_never_collected_series_is_due_immediately() {
        assert!(is_due(CollectionCadence::Weekly, None, at(2026, 8, 24, 9)));
        assert!(is_due(CollectionCadence::Daily, None, at(2026, 8, 24, 9)));
    }

    #[test]
    fn weekly_prefers_sunday_night_but_never_slips_past_eight_days() {
        // 2026-08-23 is a Sunday.
        let sunday_night = at(2026, 8, 23, 22);
        let six_days_ago = (sunday_night - chrono::Duration::days(6)).to_rfc3339();
        assert!(
            is_due(CollectionCadence::Weekly, Some(&six_days_ago), sunday_night),
            "six days old on a Sunday night: collect, and the schedule settles here"
        );
        // The same six-day-old series on a Tuesday morning is not yet due.
        let tuesday = at(2026, 8, 25, 9);
        let six_days_before_tuesday = (tuesday - chrono::Duration::days(6)).to_rfc3339();
        assert!(!is_due(
            CollectionCadence::Weekly,
            Some(&six_days_before_tuesday),
            tuesday
        ));
        // But a full week always fires, whatever day it is — a daemon asleep on
        // Sunday collects on Monday rather than skipping the week.
        let eight_days = (tuesday - chrono::Duration::days(8)).to_rfc3339();
        assert!(is_due(
            CollectionCadence::Weekly,
            Some(&eight_days),
            tuesday
        ));
    }

    #[test]
    fn daily_fires_once_a_day() {
        let now = at(2026, 8, 24, 9);
        let recent = (now - chrono::Duration::hours(3)).to_rfc3339();
        assert!(!is_due(CollectionCadence::Daily, Some(&recent), now));
        let yesterday = (now - chrono::Duration::hours(21)).to_rfc3339();
        assert!(is_due(CollectionCadence::Daily, Some(&yesterday), now));
    }

    #[tokio::test]
    async fn a_pass_collects_only_approved_series_and_records_the_points() {
        let pool = pool().await;
        let knobs = Knobs::default();
        let approved = store::bind(
            &pool,
            &knobs,
            store::BindRequest::new("p1", SourceKind::Npm, "vitest").cadence(Some(Cadence::Daily)),
        )
        .await
        .unwrap();
        store::set_status(&pool, &approved.id, SeriesStatus::Active)
            .await
            .unwrap();
        // A second series left un-approved must not be collected at all.
        store::bind(
            &pool,
            &knobs,
            store::BindRequest::new("p1", SourceKind::Npm, "vite"),
        )
        .await
        .unwrap();

        let now = at(2026, 8, 24, 9);
        let later = now + chrono::Duration::days(8);

        // Two recorded bodies: the first pass backfills from nothing, the
        // second asks a NARROWER, overlapping window anchored on the last
        // stored point. They are different URLs — a fixture that only covered
        // the first would prove nothing about the incremental pass.
        let mut bodies = Vec::new();
        for (last, today) in [
            (None, now.date_naive()),
            (Some("2026-08-23"), later.date_naive()),
        ] {
            let (since, until) = collect::incremental_window(last, today);
            for req in
                collect::plan(SourceKind::Npm, "vitest", Cadence::Daily, since, until).unwrap()
            {
                bodies.push((req.url, NPM_BODY.to_string()));
            }
        }
        let fetcher = FixtureFetcher::new(bodies);

        let report = collect_due(&pool, &fetcher, &knobs, now).await;
        assert_eq!(
            report.considered, 1,
            "the unapproved series is not even considered"
        );
        assert_eq!(report.collected, 1);
        assert_eq!(report.points_added, 2);
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        // Same day again: weekly cadence, so nothing is due and nothing is
        // fetched. The tick is not the cadence.
        let report = collect_due(&pool, &fetcher, &knobs, now).await;
        assert_eq!(report.collected, 0);
        assert_eq!(report.skipped_not_due, 1);

        // Eight days on it IS due, and re-collecting the overlap adds nothing.
        let report = collect_due(&pool, &fetcher, &knobs, later).await;
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.collected, 1);
        assert_eq!(report.points_added, 0, "re-collection is a no-op");
        assert_eq!(
            store::load_points(&pool, &approved.id).await.unwrap().len(),
            2,
            "the series did not double"
        );
    }

    #[tokio::test]
    async fn a_failed_collector_is_recorded_on_the_series_not_swallowed() {
        let pool = pool().await;
        let knobs = Knobs::default();
        let s = store::bind(
            &pool,
            &knobs,
            store::BindRequest::new("p1", SourceKind::Npm, "vitest").cadence(Some(Cadence::Daily)),
        )
        .await
        .unwrap();
        store::set_status(&pool, &s.id, SeriesStatus::Active)
            .await
            .unwrap();

        // No fixtures: every request fails.
        let fetcher = FixtureFetcher::new([]);
        let report = collect_due(&pool, &fetcher, &knobs, at(2026, 8, 24, 9)).await;
        assert_eq!(report.collected, 0);
        assert_eq!(report.errors.len(), 1);

        let summary = &store::summarize(&pool, Some("p1"), chrono::Utc::now())
            .await
            .unwrap()[0];
        assert!(
            summary.last_error.is_some(),
            "a dead collector must be visible, not silent"
        );
    }

    #[tokio::test]
    async fn equity_closes_are_not_persisted_unless_the_knob_says_so() {
        let pool = pool().await;
        let knobs = Knobs::default();
        let s = store::bind(
            &pool,
            &knobs,
            store::BindRequest::new("p1", SourceKind::EquityClose, "WE"),
        )
        .await
        .unwrap();
        store::set_status(&pool, &s.id, SeriesStatus::Active)
            .await
            .unwrap();
        let fetcher = FixtureFetcher::new([]);
        let report = collect_due(&pool, &fetcher, &knobs, at(2026, 8, 24, 9)).await;
        assert_eq!(report.collected, 0);
        assert_eq!(report.skipped_not_due, 1);
        assert!(
            report.errors.is_empty(),
            "skipping by policy is not an error: {:?}",
            report.errors
        );
    }
}
