//! Behavioural bot detection for first-party analytics — the layer the
//! User-Agent test cannot be.
//!
//! [`analytics_classify::is_bot`](super::analytics_classify::is_bot) matches
//! substrings in the User-Agent STRING, and that is all a string can tell you.
//! A headless browser driven by Playwright, or any of the commercial scraping
//! APIs built on one, sends a stock desktop Chrome UA. The string test sees a
//! human, because on the wire there is nothing else to see.
//!
//! **The incident.** On 2026-08-13/14 a headless crawler swept one project's
//! `/events/*` pages: roughly 3,700 events across 12 device hashes, each one
//! touching 80-148 distinct paths between 00:01 and 05:52 UTC, no referrer,
//! arriving through BR/IN/BD/AR/MX proxy exits. Every row was stored
//! `is_bot = 0`. Two things followed. Pageviews for the period read about 65x
//! reality. And because the crawler never reused a session, it added ~3,700
//! single-page sessions to the denominator of every session metric, forcing
//! bounce rate toward 100% and pages-per-session toward 1.0 — which is what
//! that project's growth recommendations ("95% bounce") were then computed
//! from. A wrong number that looks like a diagnosis is worse than a missing
//! one.
//!
//! **The rule.** Per `(project_id, UTC date, visitor_hash)` group:
//!
//! ```text
//! pageviews >= 10  AND  distinct sessions / pageviews >= 0.9   →   is_bot = 1
//! ```
//!
//! Why this shape and not a rate limit or a path-diversity count: the session
//! id lives in `sessionStorage`, so a real browser REUSES it for every page it
//! opens in a tab. A person who reads ten pages of a site produces one session,
//! or two if they opened a second tab — never ten. Ten-plus pageviews with a
//! near one-to-one session-per-pageview ratio is a client that throws its
//! storage away between requests, which is a fresh browser context per URL:
//! what a headless crawl does and what ordinary browsing cannot do by accident.
//! The ratio is the discriminating half; the volume floor exists only to keep
//! the rule off the short visits where a 1.0 ratio is just noise (one page,
//! one session).
//!
//! **Thresholds.** Validated against the live analytics store on 2026-08-18: at
//! `>= 10 pageviews` and `>= 0.9` sessions-per-pageview the rule selects
//! exactly the known crawl device-days and touches none of the 837 device-days
//! of ordinary 1-4 pageview traffic. Do not move either number without
//! re-running that check and saying so here.
//!
//! **Two invariants**, both load-bearing:
//!
//! * **Only ever 0 → 1.** Nothing here clears `is_bot`. The UA classifier's
//!   decisions at collect time must survive, and a human must never be silently
//!   un-flagged — or re-flagged, then un-flagged — by a later pass changing its
//!   mind. That also makes the pass idempotent: a second run over the same rows
//!   updates nothing and reports nothing.
//! * **Every row in the group**, not just the pageviews. `event` rows from the
//!   same device-day inflate funnels and conversion rates exactly as pageviews
//!   inflate traffic, and they belong to the same automated client.

use sqlx::{Pool, Sqlite};

/// Minimum pageviews in a device-day before the ratio is trusted at all.
pub const MIN_PAGEVIEWS: i64 = 10;

/// Minimum distinct-sessions-per-pageview ratio to call a device-day automated.
pub const MIN_SESSION_RATIO: f64 = 0.9;

/// How far back each pass looks, in whole UTC days.
///
/// Not "since the last pass": the Evntally rows arrived days after the traffic
/// happened, drained from the site's own relay in a backfill, so a pass keyed
/// on arrival time would have classified none of them. Re-examining a rolling
/// 30 days catches late arrivals retroactively while keeping the work bounded —
/// the alternative, rescanning all history every tick, grows without limit for
/// no benefit, because rows older than the window have already been through
/// thirty days of passes.
pub const WINDOW_DAYS: u32 = 30;

/// What one pass changed. Empty means the pass was a no-op, which is the
/// common case and must stay silent.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BehaviouralSweep {
    /// Rows moved from `is_bot = 0` to `is_bot = 1`.
    pub rows_flagged: u64,
    /// Distinct `(project, day, device)` groups that actually changed.
    pub device_days: usize,
    /// Project ids touched, in first-seen order.
    pub projects: Vec<String>,
}

impl BehaviouralSweep {
    /// Did this pass change anything a user would see?
    pub fn is_empty(&self) -> bool {
        self.rows_flagged == 0
    }
}

/// Flag automated device-days in the last `window_days` days.
///
/// Idempotent and monotonic: rows only ever move 0 → 1, so running this twice
/// flags the same rows once and reports nothing the second time. See the module
/// docs for the rule and why its thresholds are what they are.
pub async fn flag_behavioural_bots(
    pool: &Pool<Sqlite>,
    window_days: u32,
) -> Result<BehaviouralSweep, sqlx::Error> {
    let window = format!("-{window_days} days");

    // Whole UTC days on both ends. `date('now', '-30 days')` yields
    // `YYYY-MM-DD`, which is a prefix of every stored `created_at`, so the
    // lexical `>=` is exactly "this day or later" AND stays a range scan the
    // `(project_id, created_at)` index can serve. Comparing dates would mean
    // wrapping the column in `date()` and losing that.
    //
    // The metric counts every pageview in the day, including ones already
    // flagged by the UA test: they are the same client's requests, and
    // excluding them would let a crawl that half-identifies itself slip under
    // the volume floor.
    let groups: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT project_id, substr(created_at, 1, 10) AS day, visitor_hash
           FROM analytics_events
          WHERE kind = 'pageview'
            AND created_at >= date('now', ?1)
            AND visitor_hash IS NOT NULL
            AND visitor_hash <> ''
          GROUP BY project_id, day, visitor_hash
         HAVING COUNT(*) >= ?2
            AND CAST(COUNT(DISTINCT session_id) AS REAL) / COUNT(*) >= ?3",
    )
    .bind(&window)
    .bind(MIN_PAGEVIEWS)
    .bind(MIN_SESSION_RATIO)
    .fetch_all(pool)
    .await?;

    let mut sweep = BehaviouralSweep::default();
    for (project_id, day, visitor_hash) in groups {
        // No `kind` filter: the whole device-day goes, events included.
        // `is_bot = 0` is what makes this monotonic and idempotent — a row the
        // UA classifier already judged is not touched, and not counted.
        let affected = sqlx::query(
            "UPDATE analytics_events
                SET is_bot = 1
              WHERE project_id = ?1
                AND visitor_hash = ?2
                AND substr(created_at, 1, 10) = ?3
                AND is_bot = 0",
        )
        .bind(&project_id)
        .bind(&visitor_hash)
        .bind(&day)
        .execute(pool)
        .await?
        .rows_affected();

        if affected == 0 {
            continue;
        }
        sweep.rows_flagged += affected;
        sweep.device_days += 1;
        if !sweep.projects.iter().any(|p| p == &project_id) {
            sweep.projects.push(project_id);
        }
    }
    Ok(sweep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use permagent::session::spectral_schema::apply_analytics_events_schema;

    async fn mem_pool() -> Pool<Sqlite> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        apply_analytics_events_schema(&pool).await.unwrap();
        pool
    }

    /// A date inside the window, as the stored `created_at` prefix.
    fn day_offset(days_ago: i64) -> String {
        (chrono::Utc::now() - chrono::Duration::days(days_ago))
            .format("%Y-%m-%d")
            .to_string()
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert(
        pool: &Pool<Sqlite>,
        project: &str,
        visitor: &str,
        kind: &str,
        path: &str,
        session: Option<&str>,
        day: &str,
        hour: u32,
        is_bot: i64,
    ) {
        sqlx::query(
            "INSERT INTO analytics_events
                (project_id, kind, path, visitor_hash, session_id, is_bot, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(project)
        .bind(kind)
        .bind(path)
        .bind(visitor)
        .bind(session)
        .bind(is_bot)
        .bind(format!("{day}T{hour:02}:00:00.000Z"))
        .execute(pool)
        .await
        .unwrap();
    }

    /// 15 pages, each in its own session, each a different path — the shape of
    /// the 2026-08-13/14 sweep. Optionally seed some rows already UA-flagged.
    async fn seed_crawler(pool: &Pool<Sqlite>, visitor: &str, day: &str, already_flagged: usize) {
        for i in 0..15u32 {
            let flagged = i64::from((i as usize) < already_flagged);
            insert(
                pool,
                "p1",
                visitor,
                "pageview",
                &format!("/events/{i}"),
                Some(&format!("s-{visitor}-{i}")),
                day,
                i,
                flagged,
            )
            .await;
        }
    }

    async fn bot_flags(pool: &Pool<Sqlite>, visitor: &str) -> Vec<i64> {
        sqlx::query_scalar(
            "SELECT is_bot FROM analytics_events WHERE visitor_hash = ?1 ORDER BY id",
        )
        .bind(visitor)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// The incident case. A headless crawl sends a stock Chrome UA, so nothing
    /// at collect time can catch it; a fresh session per page is the tell. Its
    /// `event` rows must go too — they inflate funnels the way its pageviews
    /// inflate traffic.
    #[tokio::test]
    async fn a_fresh_session_per_pageview_is_automated_traffic() {
        let pool = mem_pool().await;
        let day = day_offset(2);
        seed_crawler(&pool, "crawler", &day, 0).await;
        // Two conversion events from the same device-day.
        for i in 0..2u32 {
            insert(
                &pool,
                "p1",
                "crawler",
                "event",
                "/events/1",
                Some(&format!("s-crawler-ev{i}")),
                &day,
                16 + i,
                0,
            )
            .await;
        }

        let sweep = flag_behavioural_bots(&pool, WINDOW_DAYS).await.unwrap();
        assert_eq!(sweep.rows_flagged, 17, "every row in the device-day");
        assert_eq!(sweep.device_days, 1);
        assert_eq!(sweep.projects, vec!["p1".to_string()]);

        let flags = bot_flags(&pool, "crawler").await;
        assert_eq!(flags.len(), 17);
        assert!(
            flags.iter().all(|&b| b == 1),
            "pageviews AND events must be flagged: {flags:?}"
        );
    }

    /// The same volume from a person. `sessionStorage` persists the session id
    /// across pages in a tab, so ten-plus pages come from one or two sessions —
    /// nowhere near the ratio. This is the test that stops the guard eating
    /// real readers.
    #[tokio::test]
    async fn a_human_reading_many_pages_is_not_flagged() {
        let pool = mem_pool().await;
        let day = day_offset(2);
        for i in 0..15u32 {
            let session = if i < 9 { "s-tab-1" } else { "s-tab-2" };
            insert(
                &pool,
                "p1",
                "human",
                "pageview",
                &format!("/events/{i}"),
                Some(session),
                &day,
                i,
                0,
            )
            .await;
        }

        let sweep = flag_behavioural_bots(&pool, WINDOW_DAYS).await.unwrap();
        assert!(sweep.is_empty(), "{sweep:?}");
        assert!(bot_flags(&pool, "human").await.iter().all(|&b| b == 0));
    }

    /// Ratio 1.0, but three pageviews. One page in one session is what almost
    /// every short visit looks like; without the volume floor the rule would
    /// flag most of the site's real traffic. 837 device-days of 1-4 pageview
    /// traffic sat under this floor in the 2026-08-18 validation.
    #[tokio::test]
    async fn a_short_visit_at_ratio_one_is_below_the_volume_floor() {
        let pool = mem_pool().await;
        let day = day_offset(2);
        for i in 0..3u32 {
            insert(
                &pool,
                "p1",
                "brief",
                "pageview",
                &format!("/events/{i}"),
                Some(&format!("s-brief-{i}")),
                &day,
                i,
                0,
            )
            .await;
        }

        let sweep = flag_behavioural_bots(&pool, WINDOW_DAYS).await.unwrap();
        assert!(sweep.is_empty(), "{sweep:?}");
        assert!(bot_flags(&pool, "brief").await.iter().all(|&b| b == 0));
    }

    /// The pass runs every drain tick. A second pass over the same rows must
    /// change nothing and, just as importantly, REPORT nothing — a log line
    /// saying "flagged 3,700 rows" every two minutes forever is a lie about
    /// what the pass did.
    #[tokio::test]
    async fn running_twice_flags_once_and_reports_nothing_the_second_time() {
        let pool = mem_pool().await;
        let day = day_offset(2);
        seed_crawler(&pool, "crawler", &day, 0).await;

        let first = flag_behavioural_bots(&pool, WINDOW_DAYS).await.unwrap();
        assert_eq!(first.rows_flagged, 15);
        assert_eq!(first.device_days, 1);

        let second = flag_behavioural_bots(&pool, WINDOW_DAYS).await.unwrap();
        assert!(second.is_empty(), "second pass must be a no-op: {second:?}");
        assert_eq!(second.device_days, 0);
        assert!(second.projects.is_empty());

        assert!(bot_flags(&pool, "crawler").await.iter().all(|&b| b == 1));
    }

    /// Rows the UA classifier already judged are left alone: the update is
    /// `is_bot = 0` only, so they are neither rewritten nor counted toward what
    /// this pass claims to have changed.
    #[tokio::test]
    async fn rows_already_flagged_by_user_agent_are_untouched_and_uncounted() {
        let pool = mem_pool().await;
        let day = day_offset(2);
        seed_crawler(&pool, "crawler", &day, 3).await;

        let sweep = flag_behavioural_bots(&pool, WINDOW_DAYS).await.unwrap();
        assert_eq!(
            sweep.rows_flagged, 12,
            "the 3 UA-flagged rows must not be re-counted"
        );
        assert_eq!(sweep.device_days, 1);
        assert!(bot_flags(&pool, "crawler").await.iter().all(|&b| b == 1));
    }

    /// The window bounds the work, not the truth: a device-day older than it
    /// has already been through thirty days of passes and is not re-examined.
    #[tokio::test]
    async fn traffic_older_than_the_window_is_not_rescanned() {
        let pool = mem_pool().await;
        seed_crawler(&pool, "old-crawler", &day_offset(45), 0).await;
        seed_crawler(&pool, "new-crawler", &day_offset(3), 0).await;

        let sweep = flag_behavioural_bots(&pool, WINDOW_DAYS).await.unwrap();
        assert_eq!(sweep.rows_flagged, 15);
        assert_eq!(sweep.device_days, 1);
        assert!(bot_flags(&pool, "new-crawler")
            .await
            .iter()
            .all(|&b| b == 1));
        assert!(bot_flags(&pool, "old-crawler")
            .await
            .iter()
            .all(|&b| b == 0));
    }

    /// A device-day is one project's. Two projects that happen to share a
    /// visitor hash must be judged, and reported, separately.
    #[tokio::test]
    async fn groups_do_not_leak_across_projects() {
        let pool = mem_pool().await;
        let day = day_offset(2);
        seed_crawler(&pool, "shared", &day, 0).await;
        // Same hash, different project, human shape.
        for i in 0..15u32 {
            insert(
                &pool,
                "p2",
                "shared",
                "pageview",
                &format!("/x/{i}"),
                Some("s-one"),
                &day,
                i,
                0,
            )
            .await;
        }

        let sweep = flag_behavioural_bots(&pool, WINDOW_DAYS).await.unwrap();
        assert_eq!(sweep.rows_flagged, 15);
        assert_eq!(sweep.projects, vec!["p1".to_string()]);

        let p2_flags: Vec<i64> = sqlx::query_scalar(
            "SELECT is_bot FROM analytics_events WHERE project_id = 'p2' ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(p2_flags.iter().all(|&b| b == 0), "{p2_flags:?}");
    }
}
