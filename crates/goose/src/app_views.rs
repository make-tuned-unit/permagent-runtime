//! Shared, read-only aggregations for data rendered by Permagent surfaces.
//!
//! The daemon routes and the agent's app-perception tool both call these
//! functions. Keeping the aggregation here prevents the app and Henry from
//! giving different answers over the same local data.

use crate::cost_router::budget::{budget_verdict, BudgetBand, BudgetCeilings, BudgetConfig};
use serde::Serialize;
use sqlx::{Pool, Row, Sqlite};
use std::collections::BTreeMap;
use std::path::Path;

// ── First-party analytics ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyticsWindow {
    Days(u32),
    AllTime,
}

impl AnalyticsWindow {
    pub fn days(days: u32) -> Self {
        Self::Days(days.clamp(1, 365))
    }

    fn since_modifier(self) -> Option<String> {
        match self {
            Self::Days(days) => Some(format!("-{days} days")),
            Self::AllTime => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RankedCount {
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankedCounts {
    pub items: Vec<RankedCount>,
    pub total: i64,
    pub limit: usize,
    pub truncated: bool,
}

/// One day of traffic. The agent could previously only see WINDOW TOTALS, so
/// "traffic dipped on the 3rd" or "the campaign spiked Tuesday" were
/// unanswerable — it had the same numbers whether the window was flat or a
/// cliff. The Grow UI already charted this; the agent's view simply never
/// carried it (reported 2026-08-04 as no daily drilldown).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AnalyticsDay {
    pub date: String,
    pub pageviews: i64,
    pub visitors: i64,
    pub events: i64,
}

#[derive(Debug, Clone)]
pub struct AnalyticsSummary {
    pub event_count: i64,
    pub pageviews: i64,
    pub unique_visitors: i64,
    pub human_events: i64,
    pub bot_events: i64,
    pub range_start: Option<String>,
    pub range_end: Option<String>,
    pub top_paths: RankedCounts,
    pub top_utm_sources: RankedCounts,
    pub top_utm_mediums: RankedCounts,
    pub top_utm_campaigns: RankedCounts,
    /// Ascending by date, one row per day that had traffic. Days with no
    /// traffic are absent rather than zero-filled — the caller knows the
    /// window, and inventing rows would imply measurement that did not happen.
    pub daily: Vec<AnalyticsDay>,
}

fn analytics_where(including_bots: bool) -> String {
    let mut clauses = vec!["(?2 IS NULL OR created_at >= datetime('now', ?2))"];
    if !including_bots {
        clauses.push("is_bot = 0");
    }
    format!(" AND {}", clauses.join(" AND "))
}

async fn ranked_analytics_counts(
    pool: &Pool<Sqlite>,
    project_id: &str,
    window: AnalyticsWindow,
    including_bots: bool,
    expression: &str,
    extra_filter: &str,
    limit: usize,
) -> anyhow::Result<RankedCounts> {
    // `expression` and `extra_filter` are fixed call-site literals, never
    // caller input. Values and the project scope remain bound parameters.
    let filter = analytics_where(including_bots);
    let since = window.since_modifier();
    let rows_sql = format!(
        "SELECT {expression} AS name, count(*) AS count
           FROM analytics_events
          WHERE project_id = ?1{extra_filter}{filter}
          GROUP BY {expression}
          ORDER BY count(*) DESC, name ASC
          LIMIT {}",
        limit.saturating_add(1)
    );
    let rows = sqlx::query_as::<_, (String, i64)>(sqlx::AssertSqlSafe(rows_sql))
        .bind(project_id)
        .bind(&since)
        .fetch_all(pool)
        .await?;

    let total_sql = format!(
        "SELECT count(DISTINCT {expression})
           FROM analytics_events
          WHERE project_id = ?1{extra_filter}{filter}"
    );
    let total = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(total_sql))
        .bind(project_id)
        .bind(&since)
        .fetch_one(pool)
        .await?;

    let truncated = rows.len() > limit;
    let items = rows
        .into_iter()
        .take(limit)
        .map(|(name, count)| RankedCount { name, count })
        .collect();
    Ok(RankedCounts {
        items,
        total,
        limit,
        truncated,
    })
}

/// Aggregate the first-party analytics view for one project.
///
/// This is the canonical computation used by both the Grow Analytics route and
/// `observe_app(surface="analytics")`. It never returns event rows or join-only
/// identifiers.
pub async fn analytics_summary(
    pool: &Pool<Sqlite>,
    project_id: &str,
    window: AnalyticsWindow,
    including_bots: bool,
    limit: usize,
) -> anyhow::Result<AnalyticsSummary> {
    let filter = analytics_where(including_bots);
    let since = window.since_modifier();

    let headline_sql = format!(
        "SELECT count(*) AS event_count,
                coalesce(sum(kind = 'pageview'), 0) AS pageviews,
                count(DISTINCT CASE WHEN kind = 'pageview' THEN visitor_hash END)
                    AS unique_visitors,
                coalesce(sum(is_bot = 0), 0) AS human_events,
                coalesce(sum(is_bot = 1), 0) AS bot_events,
                min(date(created_at)) AS range_start,
                max(date(created_at)) AS range_end
           FROM analytics_events
          WHERE project_id = ?1{filter}"
    );
    let row = sqlx::query(sqlx::AssertSqlSafe(headline_sql))
        .bind(project_id)
        .bind(&since)
        .fetch_one(pool)
        .await?;

    // Bot split reports the full measured traffic even when the headline view
    // excludes bots. This is the same "bots excluded" truth signal the Grow UI
    // shows, not a hidden filter.
    let bot_sql = "SELECT coalesce(sum(is_bot = 0), 0), coalesce(sum(is_bot = 1), 0)
           FROM analytics_events
          WHERE project_id = ?1
            AND (?2 IS NULL OR created_at >= datetime('now', ?2))";
    let (human_events, bot_events) = sqlx::query_as::<_, (i64, i64)>(bot_sql)
        .bind(project_id)
        .bind(&since)
        .fetch_one(pool)
        .await?;

    let top_paths = ranked_analytics_counts(
        pool,
        project_id,
        window,
        including_bots,
        "path",
        " AND kind = 'pageview'",
        limit,
    )
    .await?;
    let top_utm_sources = ranked_analytics_counts(
        pool,
        project_id,
        window,
        including_bots,
        "utm_source",
        " AND utm_source IS NOT NULL AND utm_source <> ''",
        limit,
    )
    .await?;
    let top_utm_mediums = ranked_analytics_counts(
        pool,
        project_id,
        window,
        including_bots,
        "utm_medium",
        " AND utm_medium IS NOT NULL AND utm_medium <> ''",
        limit,
    )
    .await?;
    let top_utm_campaigns = ranked_analytics_counts(
        pool,
        project_id,
        window,
        including_bots,
        "coalesce(utm_campaign, utm_source)",
        " AND (utm_campaign IS NOT NULL OR utm_source IS NOT NULL)",
        limit,
    )
    .await?;

    // Daily drilldown. Same window and same bot filter as the headline, so a
    // reader can add the days up and land on the totals above — a series that
    // disagreed with its own headline would be worse than none.
    let daily_sql = format!(
        "SELECT date(created_at) AS day,
                coalesce(sum(kind = 'pageview'), 0) AS pageviews,
                count(DISTINCT CASE WHEN kind = 'pageview' THEN visitor_hash END) AS visitors,
                count(*) AS events
           FROM analytics_events
          WHERE project_id = ?1{filter}
          GROUP BY date(created_at)
          ORDER BY date(created_at)"
    );
    let daily = sqlx::query(sqlx::AssertSqlSafe(daily_sql))
        .bind(project_id)
        .bind(&since)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| -> anyhow::Result<AnalyticsDay> {
            Ok(AnalyticsDay {
                date: r.try_get("day")?,
                pageviews: r.try_get("pageviews")?,
                visitors: r.try_get("visitors")?,
                events: r.try_get("events")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(AnalyticsSummary {
        event_count: row.try_get("event_count")?,
        pageviews: row.try_get("pageviews")?,
        unique_visitors: row.try_get("unique_visitors")?,
        human_events,
        bot_events,
        range_start: row.try_get("range_start")?,
        range_end: row.try_get("range_end")?,
        top_paths,
        top_utm_sources,
        top_utm_mediums,
        top_utm_campaigns,
        daily,
    })
}

// ── Spend ──────────────────────────────────────────────────────────────────

/// A lean per-session spend row read from the `sessions` rollup columns.
#[derive(Debug, Clone)]
pub struct SpendRow {
    pub id: String,
    pub name: String,
    pub working_dir: String,
    pub session_type: String,
    pub cost_usd: f64,
    pub tokens: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSpend {
    pub id: String,
    pub name: String,
    pub working_dir: String,
    pub session_type: String,
    pub cost_usd: f64,
    pub tokens: i64,
    pub updated_at: String,
    pub band: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSpend {
    pub path: String,
    pub label: String,
    pub cost_usd: f64,
    pub tokens: i64,
    pub session_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CeilingsView {
    pub soft: f64,
    pub gate: f64,
    pub hard: f64,
}

impl From<BudgetCeilings> for CeilingsView {
    fn from(c: BudgetCeilings) -> Self {
        Self {
            soft: c.soft,
            gate: c.gate,
            hard: c.hard,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetView {
    pub session: CeilingsView,
    pub task: CeilingsView,
}

impl From<BudgetConfig> for BudgetView {
    fn from(c: BudgetConfig) -> Self {
        Self {
            session: c.session.into(),
            task: c.task.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendSnapshot {
    pub running_total_usd: f64,
    pub total_tokens: i64,
    pub session_count: usize,
    pub budget: BudgetView,
    pub sessions: Vec<SessionSpend>,
    pub projects: Vec<ProjectSpend>,
}

fn band_str(b: BudgetBand) -> &'static str {
    match b {
        BudgetBand::Ok => "ok",
        BudgetBand::Soft => "soft",
        BudgetBand::Gate => "gate",
        BudgetBand::Hard => "hard",
    }
}

fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Canonical Governance Spend aggregation.
pub fn build_spend_snapshot(
    rows: Vec<SpendRow>,
    project_names: &BTreeMap<String, String>,
    cfg: &BudgetConfig,
    limit: usize,
) -> SpendSnapshot {
    let running_total_usd: f64 = rows.iter().map(|r| r.cost_usd).sum();
    let total_tokens: i64 = rows.iter().map(|r| r.tokens).sum();
    let session_count = rows.len();

    let mut by_project: BTreeMap<String, ProjectSpend> = BTreeMap::new();
    for r in &rows {
        let entry = by_project
            .entry(r.working_dir.clone())
            .or_insert_with(|| ProjectSpend {
                path: r.working_dir.clone(),
                label: project_names
                    .get(&r.working_dir)
                    .cloned()
                    .unwrap_or_else(|| basename(&r.working_dir)),
                cost_usd: 0.0,
                tokens: 0,
                session_count: 0,
            });
        entry.cost_usd += r.cost_usd;
        entry.tokens += r.tokens;
        entry.session_count += 1;
    }
    let mut projects: Vec<ProjectSpend> = by_project.into_values().collect();
    projects.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut sessions: Vec<SessionSpend> = rows
        .into_iter()
        .map(|r| {
            let band = budget_verdict(0.0, r.cost_usd, cfg).band;
            SessionSpend {
                id: r.id,
                name: r.name,
                working_dir: r.working_dir,
                session_type: r.session_type,
                cost_usd: r.cost_usd,
                tokens: r.tokens,
                updated_at: r.updated_at,
                band: band_str(band).to_string(),
            }
        })
        .collect();
    sessions.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    sessions.truncate(limit);

    SpendSnapshot {
        running_total_usd,
        total_tokens,
        session_count,
        budget: (*cfg).into(),
        sessions,
        projects,
    }
}

pub async fn read_spend_rows(pool: &Pool<Sqlite>) -> anyhow::Result<Vec<SpendRow>> {
    let rows = sqlx::query(
        "SELECT id,
                CASE WHEN name != '' THEN name ELSE description END AS label,
                working_dir, session_type,
                COALESCE(accumulated_cost_usd, 0.0) AS cost,
                COALESCE(accumulated_total_tokens, 0) AS tokens,
                updated_at
           FROM sessions
          WHERE COALESCE(accumulated_cost_usd, 0.0) > 0.0
             OR COALESCE(accumulated_total_tokens, 0) > 0
          ORDER BY cost DESC, id",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| SpendRow {
            id: row.try_get("id").unwrap_or_default(),
            name: row.try_get("label").unwrap_or_default(),
            working_dir: row.try_get("working_dir").unwrap_or_default(),
            session_type: row.try_get("session_type").unwrap_or_default(),
            cost_usd: row.try_get("cost").unwrap_or(0.0),
            tokens: row.try_get("tokens").unwrap_or(0),
            updated_at: row.try_get("updated_at").unwrap_or_default(),
        })
        .collect())
}

pub async fn read_project_names(pool: &Pool<Sqlite>) -> anyhow::Result<BTreeMap<String, String>> {
    let rows = sqlx::query("SELECT root_path, name FROM projects WHERE root_path IS NOT NULL")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let path: String = row.try_get("root_path").ok()?;
            let name: String = row.try_get("name").ok()?;
            if path.is_empty() {
                None
            } else {
                Some((path, name))
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn analytics_summary_is_aggregate_bot_filtered_and_explicitly_capped() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE analytics_events (
                project_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                path TEXT NOT NULL,
                visitor_hash TEXT,
                is_bot INTEGER NOT NULL,
                utm_source TEXT,
                utm_medium TEXT,
                utm_campaign TEXT,
                created_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        for row in [
            ("pageview", "/one", "v1", 0, "newsletter", "email", "launch"),
            ("pageview", "/one", "v1", 0, "newsletter", "email", "launch"),
            ("pageview", "/two", "v2", 0, "search", "organic", ""),
            ("event", "/one", "v1", 0, "", "", ""),
            ("event", "/two", "v2", 0, "", "", ""),
            ("pageview", "/bot", "bot", 1, "", "", ""),
        ] {
            sqlx::query(
                "INSERT INTO analytics_events
                    (project_id, kind, path, visitor_hash, is_bot, utm_source,
                     utm_medium, utm_campaign, created_at)
                 VALUES ('p1', ?, ?, ?, ?, nullif(?, ''), nullif(?, ''),
                         nullif(?, ''), '2026-07-30T12:00:00Z')",
            )
            .bind(row.0)
            .bind(row.1)
            .bind(row.2)
            .bind(row.3)
            .bind(row.4)
            .bind(row.5)
            .bind(row.6)
            .execute(&pool)
            .await
            .unwrap();
        }

        let summary = analytics_summary(&pool, "p1", AnalyticsWindow::AllTime, false, 1)
            .await
            .unwrap();
        assert_eq!(summary.event_count, 5);
        assert_eq!(summary.pageviews, 3);
        assert_eq!(summary.unique_visitors, 2);
        assert_eq!(summary.human_events, 5);
        assert_eq!(summary.bot_events, 1);
        assert_eq!(summary.range_start.as_deref(), Some("2026-07-30"));
        assert_eq!(summary.top_paths.items[0].name, "/one");
        assert_eq!(summary.top_paths.items[0].count, 2);
        assert_eq!(summary.top_paths.total, 2);
        assert!(summary.top_paths.truncated);
        assert_eq!(summary.top_utm_campaigns.items[0].name, "launch");
    }

    #[test]
    fn spend_snapshot_matches_governance_rollup_and_caps_sessions() {
        let rows = vec![
            SpendRow {
                id: "s1".into(),
                name: "First".into(),
                working_dir: "/projects/one".into(),
                session_type: "user".into(),
                cost_usd: 1.25,
                tokens: 100,
                updated_at: "2026-07-30T00:00:00Z".into(),
            },
            SpendRow {
                id: "s2".into(),
                name: "Second".into(),
                working_dir: "/projects/two".into(),
                session_type: "scheduled".into(),
                cost_usd: 3.75,
                tokens: 300,
                updated_at: "2026-07-30T01:00:00Z".into(),
            },
        ];
        let names = BTreeMap::from([
            ("/projects/one".to_string(), "One".to_string()),
            ("/projects/two".to_string(), "Two".to_string()),
        ]);
        let snapshot = build_spend_snapshot(rows, &names, &BudgetConfig::default(), 1);
        assert_eq!(snapshot.running_total_usd, 5.0);
        assert_eq!(snapshot.total_tokens, 400);
        assert_eq!(snapshot.session_count, 2);
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.sessions[0].name, "Second");
        assert_eq!(snapshot.projects.len(), 2);
        assert_eq!(snapshot.projects[0].label, "Two");
    }

    #[tokio::test]
    async fn spend_read_uses_session_rollups_and_omits_unmeasured_sessions() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                working_dir TEXT NOT NULL,
                session_type TEXT NOT NULL,
                accumulated_cost_usd REAL,
                accumulated_total_tokens INTEGER,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions VALUES
                ('measured', 'Measured', '', '/p', 'user', 1.5, 42, '2026-07-30'),
                ('empty', 'Empty', '', '/p', 'user', 0.0, 0, '2026-07-30')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let rows = read_spend_rows(&pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "measured");
        assert_eq!(rows[0].cost_usd, 1.5);
        assert_eq!(rows[0].tokens, 42);
    }
}
