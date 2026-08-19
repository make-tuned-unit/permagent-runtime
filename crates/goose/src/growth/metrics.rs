//! Pre-registerable metrics and the closed measurement windows they are read
//! over.
//!
//! Everything else in `analytics` land measures a *trailing* window:
//! `created_at >= datetime('now', '-30 days')` (app_views.rs:81,
//! analytics_attribution.rs:43, growth_actions.rs:633). None of those can
//! express "the seven days ending the day before this action was verified",
//! which is exactly what a before/after comparison needs. This module adds the
//! closed `[start, end)` form and nothing else.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

/// Named event a relay may emit when an answer engine sent the visit. Must stay
/// identical to the drain's spelling; `analytics_attribution` re-exports this
/// one rather than declaring a second copy.
pub const ANSWER_ENGINE_VISIT_EVENT: &str = "answer_engine_visit";

/// Referrer hosts that mean a generated answer cited this site. Distinct from
/// ordinary search: the content was quoted, not merely ranked.
///
/// This lives here, in the shared crate, rather than beside the prompt that
/// renders it. `render_summary` (goose-server) labels a project's AEO signal
/// from this list and [`crate::growth::pooled::segment_for`] (goose) decides a
/// project's dominant acquisition channel from it; two copies would let the
/// brief tell the model "answer engines are your biggest source" while the
/// segment those results are pooled against says "search", and neither line
/// would look wrong on its own.
pub const ANSWER_ENGINE_HOSTS: &[&str] = &[
    "chatgpt.com",
    "chat.openai.com",
    "perplexity.ai",
    "claude.ai",
    "copilot.microsoft.com",
    "gemini.google.com",
    "you.com",
    "phind.com",
];

/// Does this path hold written content — a post, a guide, an article?
///
/// One definition for the same reason as the list above: `render_summary` uses
/// it to tell the model which pages are worth expanding, and
/// [`crate::growth::pooled::segment_for`] uses it to decide whether a project
/// is a content site at all. If the two ever drifted, an action would be
/// pooled against a segment the brief never described.
pub fn is_content_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains("/blog") || p.contains("/guide") || p.contains("/article") || p.contains("/post")
}

/// The whole-week windows an action is measured over, shortest first (proposal
/// "Open decisions" 2: 7/14/28 evaluated progressively, early ones provisional).
pub const WINDOW_DAYS: [u32; 3] = [7, 14, 28];

/// How much history the variance estimate looks back over. Twelve weeks is the
/// most the 400-day analytics retention (analytics_drain.rs:38) can be relied
/// on to hold for an action verified a while ago, and enough weeks that a
/// sample standard deviation means something.
pub const HISTORY_WEEKS: u32 = 12;

/// A metric an action is allowed to pre-register against.
///
/// Deliberately a closed set. `AnalyticsSummary` carries fifteen fields
/// (growth_actions.rs:127) but most cannot carry a verdict: `device_signatures`
/// merges people sharing a browser build so a delta in it is uninterpretable
/// (growth_actions.rs:198), `bots_excluded` is a filter diagnostic, and
/// `period_days`/`days_with_traffic` are properties of the window rather than
/// outcomes. Accepting an arbitrary string here would make pre-registration
/// decorative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetMetric {
    Pageviews,
    Sessions,
    AeoVisits,
    BounceRate,
}

impl TargetMetric {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pageviews" | "pageview" | "views" => Ok(Self::Pageviews),
            "sessions" | "session" => Ok(Self::Sessions),
            "aeo_visits" | "aeo" | "answer_engine_visits" => Ok(Self::AeoVisits),
            "bounce_rate" | "bounce" => Ok(Self::BounceRate),
            other => Err(format!(
                "\"{other}\" is not a measurable target. Choose one of: pageviews, sessions, \
                 aeo_visits, bounce_rate."
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pageviews => "pageviews",
            Self::Sessions => "sessions",
            Self::AeoVisits => "aeo_visits",
            Self::BounceRate => "bounce_rate",
        }
    }

    /// Is this a rate rather than a count? Counts get the Poisson floor; a rate
    /// is bounded in [0,1] and its noise depends on its denominator, so it gets
    /// a proportion test instead ([`crate::growth::power::ratio_power`]).
    pub fn is_ratio(self) -> bool {
        matches!(self, Self::BounceRate)
    }
}

/// Which way the action claimed the metric would move. Recorded before the
/// verdict so "it moved, therefore I was right" is not available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetDir {
    Up,
    Down,
}

impl TargetDir {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "up" | "increase" | "higher" => Ok(Self::Up),
            "down" | "decrease" | "lower" => Ok(Self::Down),
            other => Err(format!(
                "\"{other}\" is not a direction. Use \"up\" or \"down\"."
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }

    /// Does `delta` move the way the action claimed it would?
    pub fn agrees_with(self, delta: f64) -> bool {
        match self {
            Self::Up => delta > 0.0,
            Self::Down => delta < 0.0,
        }
    }
}

/// One closed measurement window and what the pre-registered metric read over
/// it. Serialised verbatim into `growth_actions.baseline_json` and into
/// `growth_action_outcomes.before_json` / `after_json`, so a verdict can be
/// re-derived later from the row alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricWindow {
    /// Inclusive UTC date, `YYYY-MM-DD`.
    pub start: String,
    /// Exclusive UTC date, `YYYY-MM-DD`.
    pub end: String,
    pub days: u32,
    pub metric: TargetMetric,
    /// The metric's value: a count for counts, a rate in [0,1] for ratios.
    pub value: f64,
    /// What the value rests on — the count itself, or the session denominator
    /// for a rate. The power check needs this and a bare `value` cannot supply
    /// it: a 70% bounce rate over 8 sessions and over 800 are different claims.
    pub denominator: f64,
    /// Carried for context on the card even when they are not the target.
    pub pageviews: i64,
    pub sessions: i64,
}

/// Half-open `[start, end)` in whole UTC days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl DateRange {
    pub fn days(&self) -> u32 {
        (self.end - self.start).num_days().max(0) as u32
    }
}

/// The first UTC day whose traffic is entirely post-change.
///
/// The verification day itself is half before and half after the change, so it
/// belongs to neither window and is dropped from both (see [`windows_for`]).
pub fn pivot_date(verified_at: DateTime<Utc>) -> NaiveDate {
    verified_at.date_naive() + Duration::days(1)
}

/// The `(before, after)` pair for one whole-week window, both exactly
/// `days` long.
///
/// ```text
///        before                    │ verify │        after
///   [pivot-1-days, pivot-1)        │  day   │   [pivot, pivot+days)
/// ```
///
/// Equal length and a whole multiple of 7 means both sides contain each weekday
/// the same number of times — the proposal's reason for refusing arbitrary
/// spans ("so day-of-week composition matches on both sides"). A Tuesday-heavy
/// B2B site compared over 5 days against 9 would show a swing that is nothing
/// but the calendar.
pub fn windows_for(pivot: NaiveDate, days: u32) -> (DateRange, DateRange) {
    let d = i64::from(days);
    let after = DateRange {
        start: pivot,
        end: pivot + Duration::days(d),
    };
    let before = DateRange {
        start: pivot - Duration::days(d + 1),
        end: pivot - Duration::days(1),
    };
    (before, after)
}

/// Is the `after` window complete as of `now`?
///
/// A partial week is a different mix of weekdays than the full week it is
/// compared against, and at these traffic levels that difference alone can read
/// as a decline. Nothing is judged until the last day of the window is over.
pub fn window_is_complete(pivot: NaiveDate, days: u32, now: DateTime<Utc>) -> bool {
    now.date_naive() >= pivot + Duration::days(i64::from(days))
}

fn iso(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Read one metric over a closed window.
///
/// Filters on `date(created_at)` rather than comparing the raw column against a
/// timestamp literal. `created_at` holds whatever the source sent
/// (analytics_drain.rs:353 preserves it verbatim), so the same table mixes
/// `…T10:00:00Z` and `…T10:00:00+00:00`; those two sort differently as strings
/// but identically through `date()`. Windows here are whole days anyway, so the
/// only cost is that the `(project_id, created_at)` index narrows by project
/// rather than by both — acceptable at per-project volumes, where a wrong
/// boundary would not be.
pub async fn load_window(
    pool: &Pool<Sqlite>,
    project_id: &str,
    metric: TargetMetric,
    range: DateRange,
) -> anyhow::Result<MetricWindow> {
    let (start, end) = (iso(range.start), iso(range.end));

    // Bots are excluded everywhere, as they are in the generator's own summary
    // (growth_actions.rs:632). A bot wave inside one window and not the other is
    // one of the noise sources the proposal names.
    let pageviews: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM analytics_events
          WHERE project_id = ?1 AND kind = 'pageview' AND is_bot = 0
            AND date(created_at) >= ?2 AND date(created_at) < ?3",
    )
    .bind(project_id)
    .bind(&start)
    .bind(&end)
    .fetch_one(pool)
    .await?;

    // Sessions and single-pageview sessions in one pass — bounce_rate needs both
    // and they must come from the same scan or the rate can exceed 1.
    let (sessions, bounced): (i64, i64) = sqlx::query_as(
        "SELECT count(*), coalesce(sum(views = 1), 0) FROM (
           SELECT session_id, count(*) AS views FROM analytics_events
            WHERE project_id = ?1 AND kind = 'pageview' AND is_bot = 0
              AND session_id IS NOT NULL AND session_id <> ''
              AND date(created_at) >= ?2 AND date(created_at) < ?3
            GROUP BY session_id)",
    )
    .bind(project_id)
    .bind(&start)
    .bind(&end)
    .fetch_one(pool)
    .await?;

    let (value, denominator) = match metric {
        TargetMetric::Pageviews => (pageviews as f64, pageviews as f64),
        TargetMetric::Sessions => (sessions as f64, sessions as f64),
        TargetMetric::AeoVisits => {
            let aeo: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM analytics_events
                  WHERE project_id = ?1 AND kind = 'event' AND is_bot = 0 AND name = ?4
                    AND date(created_at) >= ?2 AND date(created_at) < ?3",
            )
            .bind(project_id)
            .bind(&start)
            .bind(&end)
            .bind(ANSWER_ENGINE_VISIT_EVENT)
            .fetch_one(pool)
            .await?;
            (aeo as f64, aeo as f64)
        }
        // Zero sessions gives 0.0 with a zero denominator, which `judge` reads
        // as "no baseline" rather than as a 0% bounce rate. Returning None here
        // instead would force every caller to re-derive that distinction.
        TargetMetric::BounceRate => {
            let rate = if sessions > 0 {
                bounced as f64 / sessions as f64
            } else {
                0.0
            };
            (rate, sessions as f64)
        }
    };

    Ok(MetricWindow {
        start,
        end,
        days: range.days(),
        metric,
        value,
        denominator,
        pageviews,
        sessions,
    })
}

/// Whole-week totals of `metric` for the `weeks` weeks immediately before
/// `end`, oldest first.
///
/// Zero-filled by construction: a week with no traffic contributes a `0.0`, not
/// an absent entry. `AnalyticsSummary.daily` omits empty days (app_views.rs:74)
/// and summing that series into weeks would silently drop the quiet ones —
/// which is exactly where an intermittent project's variance lives. Dropping
/// them makes the project look far steadier than it is, and a too-small
/// variance estimate manufactures verdicts.
pub async fn weekly_history(
    pool: &Pool<Sqlite>,
    project_id: &str,
    metric: TargetMetric,
    end: NaiveDate,
    weeks: u32,
) -> anyhow::Result<Vec<f64>> {
    // Weeks that predate the project's first recorded event are NOT zeros — they
    // are absence of data, and the difference matters enormously here.
    //
    // Pushing them unconditionally zero-padded every history to exactly `weeks`
    // entries. Three things followed (adversarial review, 2026-08-14):
    // MIN_HISTORY_WEEKS could never fire because the slice was always full;
    // `weeks_observed` always reported 12 no matter how long the project had
    // existed; and the fabricated zeros inflated the standard deviation, so a
    // steady 1000-views/week project five weeks post-install got a 143%
    // detection threshold instead of 14% — locking it out of verdicts and
    // telling it a reason that was untrue.
    //
    // A verdict that states numbers it does not rest on is the failure this
    // whole module exists to prevent, so the history now reports only weeks the
    // project was actually being measured.
    let first_event = earliest_event_date(pool, project_id)
        .await?
        .and_then(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok());

    let mut out = Vec::with_capacity(weeks as usize);
    for i in (1..=i64::from(weeks)).rev() {
        let range = DateRange {
            start: end - Duration::days(i * 7),
            end: end - Duration::days((i - 1) * 7),
        };
        // Skip a week that ended before the project produced any events at all.
        if let Some(first) = first_event {
            if range.end <= first {
                continue;
            }
        }
        out.push(load_window(pool, project_id, metric, range).await?.value);
    }
    Ok(out)
}

/// Earliest event date recorded for the project, `YYYY-MM-DD`.
///
/// A `before` window that starts before this is not a measurement of a quiet
/// period — it is a measurement of a period the database never saw, either
/// because the relay was not installed yet or because the 400-day prune
/// (analytics_drain.rs:38) removed it. The two are indistinguishable from the
/// counts alone, so the caller must refuse rather than compare.
pub async fn earliest_event_date(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> anyhow::Result<Option<String>> {
    Ok(sqlx::query_scalar::<_, Option<String>>(
        "SELECT min(date(created_at)) FROM analytics_events WHERE project_id = ?1",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn only_measurable_metrics_are_pre_registerable() {
        assert_eq!(TargetMetric::parse("sessions"), Ok(TargetMetric::Sessions));
        assert_eq!(
            TargetMetric::parse("Bounce_Rate"),
            Ok(TargetMetric::BounceRate)
        );
        // The three the survey ruled uninterpretable must not sneak through.
        for bad in [
            "device_signatures",
            "bots_excluded",
            "days_with_traffic",
            "",
        ] {
            let err = TargetMetric::parse(bad).unwrap_err();
            assert!(err.contains("not a measurable target"), "{bad}: {err}");
            assert!(
                err.contains("pageviews"),
                "{bad}: {err} must name the choices"
            );
        }
    }

    /// Both sides must be the same length and a whole number of weeks, or the
    /// day-of-week composition differs and the calendar shows up as an effect.
    #[test]
    fn windows_are_equal_length_whole_weeks_and_skip_the_verify_day() {
        let pivot = pivot_date(at("2026-08-11T14:30:00Z"));
        assert_eq!(pivot, date("2026-08-12"));

        for days in WINDOW_DAYS {
            let (before, after) = windows_for(pivot, days);
            assert_eq!(before.days(), days, "{days}d before");
            assert_eq!(after.days(), days, "{days}d after");
            assert_eq!(days % 7, 0, "{days} is not whole weeks");
            // The verification day (2026-08-11) is in neither window: it is
            // half pre-change traffic and half post-change.
            assert_eq!(before.end, date("2026-08-11"));
            assert_eq!(after.start, date("2026-08-12"));
        }
    }

    #[test]
    fn a_window_is_not_judged_until_its_last_day_is_over() {
        let pivot = date("2026-08-12");
        assert!(!window_is_complete(pivot, 7, at("2026-08-18T23:59:00Z")));
        assert!(window_is_complete(pivot, 7, at("2026-08-19T00:01:00Z")));
        assert!(!window_is_complete(pivot, 28, at("2026-09-08T00:00:00Z")));
        assert!(window_is_complete(pivot, 28, at("2026-09-09T00:00:00Z")));
    }

    /// Pins the single definition both crates now share. Before this const
    /// existed the goose-server summary carried its own inline closure, so a
    /// path could be "content" to the brief and not to the segment it was
    /// pooled against.
    #[test]
    fn is_content_path_matches_what_the_summary_calls_content() {
        for content in [
            "/blog/x",
            "/guides/y",
            "/article/z",
            "/posts/1",
            "/BLOG/Caps",
        ] {
            assert!(is_content_path(content), "{content} should be content");
        }
        for not_content in ["/", "/pricing", "/app/dashboard", "/about"] {
            assert!(
                !is_content_path(not_content),
                "{not_content} should not be content"
            );
        }
    }

    #[test]
    fn direction_reads_the_sign_not_the_size() {
        assert!(TargetDir::Up.agrees_with(0.1));
        assert!(!TargetDir::Up.agrees_with(-0.1));
        assert!(TargetDir::Down.agrees_with(-40.0));
        // A flat delta agrees with neither claim.
        assert!(!TargetDir::Up.agrees_with(0.0));
        assert!(!TargetDir::Down.agrees_with(0.0));
    }
}
