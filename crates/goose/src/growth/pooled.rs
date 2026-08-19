//! Cross-project pooled learning: what a whole category of action has done on
//! the OTHER active projects, segmented before it is pooled.
//!
//! The proposal's argument for it, unchanged: "This is what rescues the
//! low-traffic projects. One action on one small project is underpowered
//! forever; the *same strategy* tried across nineteen projects has N=19, and
//! that is a sample worth reasoning about."
//!
//! And its warning, which shapes every function here: "projects are not
//! exchangeable … pooling naively produces textbook Simpson's paradox — an
//! aggregate that says 'helped' while quietly failing on the segment you are
//! about to apply it to." So the aggregate and the segment are always reported
//! side by side, never merged into one number, and the model is told to trust
//! the segment where they disagree.
//!
//! Everything here is derived from `growth_action_outcomes`, which is computed
//! arithmetic over the projects' own analytics. No part of a transfer claim is
//! ever authored by a model — that would be the self-assessed prose the whole
//! feature exists to replace.

use super::metrics::{is_content_path, ANSWER_ENGINE_HOSTS, ANSWER_ENGINE_VISIT_EVENT};
use chrono::{DateTime, Duration, Utc};
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;

/// How far back a project's shape is read. Twelve weeks, matching
/// [`super::metrics::HISTORY_WEEKS`], so the tier a project is placed in and
/// the variance its own verdicts are judged against describe the same period.
const SEGMENT_DAYS: i64 = 84;
const SEGMENT_WEEKS: f64 = 12.0;

/// Weekly pageviews, bucketed. The boundaries are the proposal's own worked
/// examples ("single-page sites", "content sites, ~500 views/wk").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficTier {
    /// Under 100 pageviews a week — nothing is individually measurable here,
    /// which is precisely the case pooling exists for.
    Low,
    /// 100 to 300 a week.
    Mid,
    /// 300 a week or more.
    High,
}

/// What kind of site this is, as its own traffic describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteShape {
    SinglePage,
    Content,
    App,
}

/// Where the visits come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Direct,
    Search,
    AnswerEngine,
    Social,
    Referral,
}

impl TrafficTier {
    fn label(self) -> &'static str {
        match self {
            Self::Low => "under 100 views/wk",
            Self::Mid => "100-300 views/wk",
            Self::High => "300+ views/wk",
        }
    }
}

impl SiteShape {
    fn label(self) -> &'static str {
        match self {
            Self::SinglePage => "single-page site",
            Self::Content => "content site",
            Self::App => "app",
        }
    }
}

impl Channel {
    fn label(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Search => "search",
            Self::AnswerEngine => "answer engines",
            Self::Social => "social",
            Self::Referral => "referrals",
        }
    }
}

/// The attributes that plausibly moderate whether a strategy transfers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectSegment {
    pub tier: TrafficTier,
    pub shape: SiteShape,
    pub channel: Channel,
}

impl ProjectSegment {
    /// The human-readable segment, e.g. `content site, 300+ views/wk, mostly
    /// search`. This is what the card shows the user, so it names all three
    /// axes even though only two decide comparability.
    pub fn label(&self) -> String {
        format!(
            "{}, {}, mostly {}",
            self.shape.label(),
            self.tier.label(),
            self.channel.label()
        )
    }

    /// Would a result from `other` plausibly transfer to this project?
    ///
    /// Traffic tier and site shape only. The proposal names a third axis —
    /// dominant acquisition channel — and it is computed and shown in
    /// [`Self::label`] so the user can audit it, but it is deliberately not a
    /// comparability test: across fewer than twenty projects a third axis makes
    /// almost every segment N<=1, and a segment of one cannot moderate
    /// anything. It would replace a weak signal with a number that only looks
    /// like one. The proposal's own worked examples segment on exactly these
    /// two.
    pub fn comparable_to(&self, other: &Self) -> bool {
        self.tier == other.tier && self.shape == other.shape
    }
}

fn channel_of(raw: &str) -> Channel {
    let s = raw.trim().to_ascii_lowercase();
    if s.is_empty() {
        return Channel::Direct;
    }
    if ANSWER_ENGINE_HOSTS.iter().any(|host| s.contains(host)) {
        return Channel::AnswerEngine;
    }
    for engine in ["google", "bing", "duckduckgo", "yahoo", "ecosia", "brave"] {
        if s.contains(engine) {
            return Channel::Search;
        }
    }
    for social in [
        "twitter",
        "x.com",
        "reddit",
        "facebook",
        "linkedin",
        "instagram",
        "ycombinator",
        "mastodon",
        "bsky",
    ] {
        if s.contains(social) {
            return Channel::Social;
        }
    }
    Channel::Referral
}

/// Place a project in its segment from its own last twelve weeks of traffic.
///
/// Read rather than configured: a segment the user typed would be a claim about
/// the project, and this has to be a measurement of it, or the pooling it feeds
/// is sorting by opinion.
pub async fn segment_for(
    pool: &Pool<Sqlite>,
    project_id: &str,
    now: DateTime<Utc>,
) -> ProjectSegment {
    let since = (now - Duration::days(SEGMENT_DAYS))
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    // (a) Shape and tier, from the pages that were viewed. LIMIT 200 bounds the
    // read on a site with a long tail of URLs; the tail is by definition small
    // and cannot change which side of a tier boundary the project falls on.
    let paths = sqlx::query_as::<_, (String, i64)>(
        "SELECT path, count(*) FROM analytics_events
          WHERE project_id = ?1 AND is_bot = 0 AND kind = 'pageview'
            AND date(created_at) >= ?2
          GROUP BY path ORDER BY count(*) DESC LIMIT 200",
    )
    .bind(project_id)
    .bind(&since)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let total: i64 = paths.iter().map(|(_, c)| *c).sum();
    let content: i64 = paths
        .iter()
        .filter(|(p, _)| is_content_path(p))
        .map(|(_, c)| *c)
        .sum();
    let weekly = total as f64 / SEGMENT_WEEKS;
    let tier = if weekly < 100.0 {
        TrafficTier::Low
    } else if weekly < 300.0 {
        TrafficTier::Mid
    } else {
        TrafficTier::High
    };
    // Two paths or fewer is a single-page site whatever those paths are named:
    // a one-page site whose only route happens to be `/blog` is not a content
    // site, and calling it one would pool it with sites that have somewhere to
    // interlink to.
    let shape = if paths.len() <= 2 {
        SiteShape::SinglePage
    } else if total > 0 && content * 4 >= total {
        SiteShape::Content
    } else {
        SiteShape::App
    };

    // (b) Dominant channel, from the same source resolution the analytics
    // rollup uses: an explicit utm_source wins, else the referrer, else direct.
    let mut buckets: HashMap<Channel, i64> = HashMap::new();
    let sources = sqlx::query_as::<_, (String, i64)>(
        "SELECT coalesce(nullif(utm_source, ''), referrer, ''), count(*) FROM analytics_events
          WHERE project_id = ?1 AND is_bot = 0 AND date(created_at) >= ?2
          GROUP BY 1 ORDER BY count(*) DESC LIMIT 20",
    )
    .bind(project_id)
    .bind(&since)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (raw, count) in sources {
        *buckets.entry(channel_of(&raw)).or_insert(0) += count;
    }

    // (c) An answer engine that stripped its referrer still announces itself
    // through the named event, and those visits are the whole reason the
    // AnswerEngine bucket exists — leaving them out would report a site cited
    // by ChatGPT as "mostly direct".
    let aeo: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM analytics_events
          WHERE project_id = ?1 AND is_bot = 0 AND kind = 'event' AND name = ?3
            AND date(created_at) >= ?2",
    )
    .bind(project_id)
    .bind(&since)
    .bind(ANSWER_ENGINE_VISIT_EVENT)
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    if aeo > 0 {
        *buckets.entry(Channel::AnswerEngine).or_insert(0) += aeo;
    }

    let channel = buckets
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(channel, _)| channel)
        .unwrap_or(Channel::Direct);

    ProjectSegment {
        tier,
        shape,
        channel,
    }
}

/// One measured action on another project, kept so a transfer claim can name
/// where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct PoolExample {
    pub project_id: String,
    pub project_name: String,
    pub title: String,
    pub verdict: String,
    pub delta_pct: Option<f64>,
    pub segment_label: String,
    /// Is the project it came from in the same tier and shape as the target?
    pub comparable: bool,
}

/// What one category of action has done elsewhere, aggregate and segment.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryPool {
    pub category: String,
    /// Distinct OTHER active projects that measured this category.
    pub projects: usize,
    pub helped: usize,
    pub hindered: usize,
    pub no_effect: usize,
    pub median_delta_pct: Option<f64>,
    pub segment_projects: usize,
    pub segment_helped: usize,
    pub segment_hindered: usize,
    pub segment_no_effect: usize,
    /// At most three, comparable projects first.
    pub examples: Vec<PoolExample>,
}

impl CategoryPool {
    fn measured(&self) -> usize {
        self.helped + self.hindered + self.no_effect
    }
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    })
}

/// One joined `(outcome, action, project)` row, in SELECT order.
///
/// Named because the eight-column tuple is unreadable at the call site and
/// because the destructuring below has to match this order exactly — a silent
/// swap of `title` and `project_id` would attribute every result to the wrong
/// project, which is the one error a provenance line cannot survive.
type OutcomeJoin = (
    String,         // o.action_id
    i64,            // o.window_days
    String,         // o.verdict
    Option<f64>,    // o.delta_pct
    Option<String>, // a.category
    String,         // a.title
    String,         // a.project_id
    String,         // p.name
);

/// Pool every other active project's measured outcomes by category.
///
/// Excludes the project being advised (its own results already reach the prompt
/// through `store::render_learning`, and counting them twice would let one
/// project's single result look like corroboration from elsewhere) and every
/// paused or archived project (a shelved project's numbers are stale by
/// definition and nobody is watching them).
pub async fn pool_by_category(
    pool: &Pool<Sqlite>,
    exclude_project_id: &str,
    target: &ProjectSegment,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<CategoryPool>> {
    let rows = sqlx::query_as::<_, OutcomeJoin>(
        "SELECT o.action_id, o.window_days, o.verdict, o.delta_pct,
                a.category, a.title, a.project_id, p.name
           FROM growth_action_outcomes o
           JOIN growth_actions a ON a.id = o.action_id
           JOIN projects p ON p.id = a.project_id
          WHERE p.status = 'active'
            AND a.project_id <> ?1
            AND o.verdict IN ('helped', 'hindered', 'no_effect')
          ORDER BY o.window_days DESC, o.judged_at DESC",
    )
    .bind(exclude_project_id)
    .fetch_all(pool)
    .await?;

    // ONE row per action, longest window first — the same guard
    // `store::learnable_outcomes` carries, for the same reason recorded in the
    // 2026-08-14 review: three windows of one change is one thing tried, and
    // counting all three would report a 3x sample size to a prompt whose entire
    // purpose is stopping "worked once" becoming "works".
    let mut seen = std::collections::HashSet::new();
    let mut kept = Vec::new();
    for row in rows {
        if seen.insert(row.0.clone()) {
            kept.push(row);
        }
    }

    // Segment each contributing project once. A project with five measured
    // actions would otherwise pay for five identical three-query reads.
    let mut segments: HashMap<String, ProjectSegment> = HashMap::new();
    for row in &kept {
        let project_id = &row.6;
        if !segments.contains_key(project_id) {
            segments.insert(project_id.clone(), segment_for(pool, project_id, now).await);
        }
    }

    struct Acc {
        pool: CategoryPool,
        projects: std::collections::HashSet<String>,
        segment_projects: std::collections::HashSet<String>,
        deltas: Vec<f64>,
    }

    let mut by_category: HashMap<String, Acc> = HashMap::new();
    for (_, _, verdict, delta_pct, category, title, project_id, project_name) in kept {
        let category = category.unwrap_or_else(|| "uncategorised".to_string());
        let segment = segments.get(&project_id).copied().unwrap_or(*target);
        let comparable = target.comparable_to(&segment);
        let entry = by_category.entry(category.clone()).or_insert_with(|| Acc {
            pool: CategoryPool {
                category: category.clone(),
                projects: 0,
                helped: 0,
                hindered: 0,
                no_effect: 0,
                median_delta_pct: None,
                segment_projects: 0,
                segment_helped: 0,
                segment_hindered: 0,
                segment_no_effect: 0,
                examples: Vec::new(),
            },
            projects: std::collections::HashSet::new(),
            segment_projects: std::collections::HashSet::new(),
            deltas: Vec::new(),
        });

        entry.projects.insert(project_id.clone());
        match verdict.as_str() {
            "helped" => entry.pool.helped += 1,
            "hindered" => entry.pool.hindered += 1,
            _ => entry.pool.no_effect += 1,
        }
        if comparable {
            entry.segment_projects.insert(project_id.clone());
            match verdict.as_str() {
                "helped" => entry.pool.segment_helped += 1,
                "hindered" => entry.pool.segment_hindered += 1,
                _ => entry.pool.segment_no_effect += 1,
            }
        }
        if let Some(delta) = delta_pct {
            entry.deltas.push(delta);
        }
        entry.pool.examples.push(PoolExample {
            project_id,
            project_name,
            title,
            verdict,
            delta_pct,
            segment_label: segment.label(),
            comparable,
        });
    }

    let mut out: Vec<CategoryPool> = by_category
        .into_values()
        .map(|mut acc| {
            acc.pool.projects = acc.projects.len();
            acc.pool.segment_projects = acc.segment_projects.len();
            acc.pool.median_delta_pct = median(acc.deltas);
            // Comparable examples first: an example from a project like this one
            // is the one the user can actually weigh.
            acc.pool
                .examples
                .sort_by_key(|e| (!e.comparable, e.title.clone()));
            acc.pool.examples.truncate(3);
            acc.pool
        })
        .collect();
    // Most-measured category first: it is the one carrying the most evidence,
    // and the brief has a finite amount of the model's attention.
    out.sort_by(|a, b| {
        b.measured()
            .cmp(&a.measured())
            .then_with(|| a.category.cmp(&b.category))
    });
    Ok(out)
}

fn pct(delta: f64) -> String {
    format!(
        "{}{:.0}%",
        if delta >= 0.0 { "+" } else { "-" },
        delta.abs() * 100.0
    )
}

/// The pooled record as the generation prompt sees it. `None` when nothing
/// elsewhere has been measured, so the brief carries no empty heading.
pub fn render_pool(pools: &[CategoryPool], target: &ProjectSegment) -> Option<String> {
    if pools.is_empty() {
        return None;
    }
    let mut out = String::from(
        "Across your other active projects, by category (one measured action per row, longest \
         window):\n",
    );
    for pool in pools {
        let median = match pool.median_delta_pct {
            Some(delta) => format!("; median {}", pct(delta)),
            None => String::new(),
        };
        out.push_str(&format!(
            "- {} — tried on {} project(s): helped {}, no effect {}, hindered {}{median}\n",
            pool.category, pool.projects, pool.helped, pool.no_effect, pool.hindered,
        ));
        if pool.segment_projects == 0 {
            out.push_str("  No project like this one has tried it.\n");
        } else {
            out.push_str(&format!(
                "  On projects like this one ({}): helped {} of {}.\n",
                target.label(),
                pool.segment_helped,
                pool.segment_projects,
            ));
        }
        // Provenance is not decoration. A card that appears because a category
        // worked elsewhere and cannot say WHERE is a recommendation the user
        // cannot audit, which the proposal rules out explicitly.
        let provenance = pool
            .examples
            .iter()
            .map(|e| {
                let delta = match e.delta_pct {
                    Some(d) => format!(", {}", pct(d)),
                    None => String::new(),
                };
                format!(
                    "\"{}\" on {} ({}{delta})",
                    e.title, e.project_name, e.verdict
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        if !provenance.is_empty() {
            out.push_str(&format!("  Provenance: {provenance}\n"));
        }
    }
    out.push_str(
        "These are other projects' before/after readings, not experiments, and projects are not \
         exchangeable. Where the segment line disagrees with the pooled line, trust the segment. \
         If you propose something because it worked elsewhere, say so in `evidence`.\n",
    );
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::growth::store::{self, ActionSeed};
    use crate::projects::{self, CreateProject, UpdateProject};
    use crate::session::spectral_schema::init_spectral_db;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-19T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    async fn project(pool: &Pool<Sqlite>, name: &str, status: &str) -> String {
        let created = projects::create_project(
            pool,
            CreateProject {
                name: name.to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        if status != "active" {
            projects::update_project(
                pool,
                &created.id,
                UpdateProject {
                    status: Some(status.to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        created.id
    }

    async fn pageviews(pool: &Pool<Sqlite>, project_id: &str, path: &str, referrer: &str, n: u32) {
        for i in 0..n {
            sqlx::query(
                "INSERT INTO analytics_events
                    (project_id, kind, path, referrer, session_id, is_bot, created_at)
                 VALUES (?1, 'pageview', ?2, ?3, ?4, 0, '2026-08-01T00:00:00Z')",
            )
            .bind(project_id)
            .bind(path)
            .bind((!referrer.is_empty()).then_some(referrer))
            .bind(format!("{path}-{i}"))
            .execute(pool)
            .await
            .unwrap();
        }
    }

    async fn aeo_events(pool: &Pool<Sqlite>, project_id: &str, n: u32) {
        for _ in 0..n {
            sqlx::query(
                "INSERT INTO analytics_events (project_id, kind, path, name, is_bot, created_at)
                 VALUES (?1, 'event', '/', ?2, 0, '2026-08-01T00:00:00Z')",
            )
            .bind(project_id)
            .bind(ANSWER_ENGINE_VISIT_EVENT)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    /// One measured action: a row plus one outcome row per window given.
    async fn measured(
        pool: &Pool<Sqlite>,
        project_id: &str,
        title: &str,
        category: &str,
        windows: &[(i64, &str, Option<f64>)],
    ) {
        let row = store::upsert_suggested(
            pool,
            project_id,
            &ActionSeed {
                title: title.into(),
                recommendation: format!("recommendation for {title}"),
                category: Some(category.into()),
                artifact_kind: Some("prompt".into()),
                artifact: None,
                target_metric: Some("sessions".into()),
                target_dir: Some("up".into()),
            },
        )
        .await
        .unwrap();
        for &(days, verdict, delta) in windows {
            sqlx::query(
                "INSERT OR REPLACE INTO growth_action_outcomes
                    (action_id, window_days, before_json, after_json, delta_pct, verdict,
                     rationale, confounders, judged_at)
                 VALUES (?1, ?2, '{}', '{}', ?3, ?4, 'fixture', NULL, '2026-09-10T00:00:00Z')",
            )
            .bind(&row.id)
            .bind(days)
            .bind(delta)
            .bind(verdict)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    fn target() -> ProjectSegment {
        ProjectSegment {
            tier: TrafficTier::Low,
            shape: SiteShape::Content,
            channel: Channel::Direct,
        }
    }

    #[tokio::test]
    async fn a_content_site_with_light_traffic_is_segmented_as_one() {
        let pool = pool().await;
        let id = project(&pool, "Grocery notes", "active").await;
        pageviews(&pool, &id, "/blog/canadian-grocery-stores", "", 20).await;
        pageviews(&pool, &id, "/blog/coupon-codes", "", 10).await;
        pageviews(&pool, &id, "/", "", 5).await;

        let segment = segment_for(&pool, &id, now()).await;
        assert_eq!(segment.tier, TrafficTier::Low);
        assert_eq!(segment.shape, SiteShape::Content);
        assert!(
            segment
                .label()
                .starts_with("content site, under 100 views/wk, mostly "),
            "{}",
            segment.label()
        );
    }

    /// A one-page site whose only route is named like a post is still a
    /// single-page site: pooling it with real content sites would credit it
    /// with somewhere to interlink to that it does not have.
    #[tokio::test]
    async fn a_two_page_site_is_not_called_content_heavy() {
        let pool = pool().await;
        let id = project(&pool, "One pager", "active").await;
        pageviews(&pool, &id, "/blog/only-post", "", 40).await;
        pageviews(&pool, &id, "/blog/other-post", "", 40).await;

        assert_eq!(
            segment_for(&pool, &id, now()).await.shape,
            SiteShape::SinglePage
        );
    }

    #[tokio::test]
    async fn the_dominant_channel_reads_answer_engines_from_the_shared_list() {
        let pool = pool().await;
        let id = project(&pool, "Cited site", "active").await;
        pageviews(&pool, &id, "/blog/a", "https://google.com/", 7).await;
        pageviews(&pool, &id, "/blog/b", "https://chatgpt.com/", 5).await;
        aeo_events(&pool, &id, 3).await;

        // 5 referred + 3 named events beats 7 from search.
        assert_eq!(
            segment_for(&pool, &id, now()).await.channel,
            Channel::AnswerEngine
        );

        // The classifier reads `metrics::ANSWER_ENGINE_HOSTS` rather than its
        // own copy, so a host removed from that const stops being classified
        // here too — which is the property that keeps the segment and the
        // generation brief describing the same site.
        for host in ANSWER_ENGINE_HOSTS {
            assert_eq!(
                channel_of(&format!("https://{host}/thread/1")),
                Channel::AnswerEngine,
                "{host}"
            );
        }
        assert_eq!(channel_of("https://news.example.com/"), Channel::Referral);
        assert_eq!(channel_of(""), Channel::Direct);
    }

    /// The same 2026-08-14 inflation the learning path already guards: three
    /// windows of one change is one thing tried, not three.
    #[tokio::test]
    async fn pooling_counts_one_action_once_not_once_per_window() {
        let pool = pool().await;
        let other = project(&pool, "Other", "active").await;
        measured(
            &pool,
            &other,
            "Add FAQ schema",
            "seo",
            &[
                (7, "helped", Some(0.2)),
                (14, "helped", Some(0.3)),
                (28, "helped", Some(0.4)),
            ],
        )
        .await;

        let pools = pool_by_category(&pool, "target", &target(), now())
            .await
            .unwrap();
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].helped, 1, "one action is one measured strategy");
        assert_eq!(pools[0].projects, 1);
        // The longest window is the settled one and must be the survivor.
        assert_eq!(pools[0].median_delta_pct, Some(0.4));
    }

    #[tokio::test]
    async fn pooling_skips_the_target_project_and_inactive_ones() {
        let pool = pool().await;
        let target_id = project(&pool, "Target", "active").await;
        let paused = project(&pool, "Paused", "paused").await;
        let shelved = project(&pool, "Shelved", "archived").await;
        let other = project(&pool, "Other", "active").await;

        for (id, title) in [
            (&target_id, "own work"),
            (&paused, "paused work"),
            (&shelved, "shelved work"),
            (&other, "other work"),
        ] {
            measured(&pool, id, title, "seo", &[(28, "helped", Some(0.1))]).await;
        }

        let pools = pool_by_category(&pool, &target_id, &target(), now())
            .await
            .unwrap();
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].projects, 1);
        assert_eq!(
            pools[0]
                .examples
                .iter()
                .map(|e| e.title.as_str())
                .collect::<Vec<_>>(),
            vec!["other work"]
        );
    }

    /// The Simpson's-paradox guard, made visible: an aggregate that says
    /// "helped" while the projects that actually resemble this one say the
    /// opposite must show both numbers, not one.
    #[tokio::test]
    async fn the_segment_is_reported_separately_from_the_aggregate() {
        let pool = pool().await;
        let target_id = project(&pool, "Target", "active").await;
        // Two single-page projects (no analytics at all reads as single-page,
        // low traffic) — not comparable to a content site.
        for name in ["Landing A", "Landing B"] {
            let id = project(&pool, name, "active").await;
            measured(
                &pool,
                &id,
                &format!("{name} FAQ schema"),
                "seo",
                &[(28, "helped", Some(0.2))],
            )
            .await;
        }
        // One content site in the same tier and shape as the target.
        let like_us = project(&pool, "Notes", "active").await;
        pageviews(&pool, &like_us, "/blog/a", "", 20).await;
        pageviews(&pool, &like_us, "/blog/b", "", 10).await;
        pageviews(&pool, &like_us, "/", "", 5).await;
        measured(
            &pool,
            &like_us,
            "Notes FAQ schema",
            "seo",
            &[(28, "hindered", Some(-0.1))],
        )
        .await;

        let pools = pool_by_category(&pool, &target_id, &target(), now())
            .await
            .unwrap();
        assert_eq!(pools.len(), 1);
        let seo = &pools[0];
        assert_eq!((seo.helped, seo.hindered, seo.no_effect), (2, 1, 0));
        assert_eq!(seo.segment_projects, 1);
        assert_eq!((seo.segment_helped, seo.segment_hindered), (0, 1));

        let text = render_pool(&pools, &target()).unwrap();
        assert!(text.contains("helped 2, no effect 0, hindered 1"), "{text}");
        assert!(
            text.contains(
                "On projects like this one (content site, under 100 views/wk, mostly \
                           direct): helped 0 of 1."
            ),
            "{text}"
        );
        assert!(text.contains("trust the segment"), "{text}");
    }

    /// A recommendation the user cannot audit is one they cannot overrule, so
    /// every pooled claim names the project and the action it came from. Before
    /// this module existed there was no cross-project read at all — every
    /// learning query was `WHERE a.project_id = ?1`.
    #[tokio::test]
    async fn the_pool_names_the_project_each_result_came_from() {
        let pool = pool().await;
        let target_id = project(&pool, "Target", "active").await;
        let other = project(&pool, "Grocery notes", "active").await;
        measured(
            &pool,
            &other,
            "Add FAQPage markup to the grocery post",
            "aeo",
            &[(28, "helped", Some(0.18))],
        )
        .await;

        let pools = pool_by_category(&pool, &target_id, &target(), now())
            .await
            .unwrap();
        let text = render_pool(&pools, &target()).unwrap();
        assert!(text.contains("Provenance:"), "{text}");
        assert!(text.contains("Grocery notes"), "{text}");
        assert!(
            text.contains("Add FAQPage markup to the grocery post"),
            "{text}"
        );
        assert!(text.contains("+18%"), "{text}");
        assert!(render_pool(&[], &target()).is_none());
    }
}
