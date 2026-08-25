//! The series registry and its append-only points.
//!
//! Two invariants live here, and everything else is bookkeeping:
//!
//! * **One subject, one series.** `UNIQUE(project_id, source_kind, subject)` is
//!   enforced in SQL and re-binding reaches the existing row instead of forking
//!   its history.
//! * **Re-collection inserts nothing.** Points go in with `INSERT OR IGNORE` on
//!   a `(series_id, ts)` primary key, so a collector that overlaps its previous
//!   window is a no-op rather than a doubled series. That is what makes the
//!   backfill-then-increment pattern safe to run on every sweep.

use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

use super::series::{normalize_subject, Cadence, Series, SeriesStatus, SourceKind};
use super::{Approval, Knobs};

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// One observation. `ts` is a date (`YYYY-MM-DD`) for daily series and the
/// Monday of the week for weekly ones — sortable as a string, which is what the
/// primary key relies on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub ts: String,
    pub value: f64,
}

/// Why a series can or cannot be forecast, in guard order.
///
/// The order is the point, as it is in `growth::power::judge`: a series that is
/// not collecting yet is `NotBound` even if it happens to have rows, and a
/// series whose collector died is `CollectorStale` even if it has enough
/// history — forecasting stale data is worse than refusing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Approved, collecting, long enough, and fresh.
    Forecastable,
    /// Bound but never approved, or dismissed. Nothing is being collected.
    NotBound,
    /// The collector has not produced a point in three cadence periods.
    CollectorStale {
        /// Renamed explicitly: the outer struct is camelCase for the UI, and a
        /// flattened variant does not inherit that.
        #[serde(rename = "lastCollectedAt")]
        last_collected_at: Option<String>,
    },
    /// Shorter than the minimum any method is allowed to speak at.
    InsufficientHistory { points: usize, needed: usize },
}

/// A registry row plus the facts that decide whether it can be forecast.
///
/// `snapshot_only` is carried separately from the verdict because it is a
/// property of the *source*, not of this series' progress: a snapshot source
/// will reach `Forecastable` eventually, one self-accumulated point per sweep,
/// and saying "snapshot only" is how the card explains why that takes months.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesSummary {
    pub series_id: String,
    pub project_id: String,
    pub intel_id: Option<String>,
    pub source_kind: SourceKind,
    pub source_label: String,
    pub subject: String,
    pub subject_group: Option<String>,
    pub cadence: Cadence,
    pub label: String,
    pub status: SeriesStatus,
    pub points: usize,
    pub span_days: i64,
    pub first_ts: Option<String>,
    pub last_ts: Option<String>,
    pub last_collected_at: Option<String>,
    pub last_error: Option<String>,
    pub snapshot_only: bool,
    pub official_source: bool,
    #[serde(flatten)]
    pub verdict: Verdict,
}

fn row_to_series(row: &sqlx::sqlite::SqliteRow) -> Result<Series, String> {
    let kind_raw: String = row.try_get("source_kind").map_err(|e| e.to_string())?;
    let cadence_raw: String = row.try_get("cadence").map_err(|e| e.to_string())?;
    let status_raw: String = row.try_get("status").map_err(|e| e.to_string())?;
    Ok(Series {
        id: row.try_get("id").map_err(|e| e.to_string())?,
        project_id: row.try_get("project_id").map_err(|e| e.to_string())?,
        intel_id: row.try_get("intel_id").map_err(|e| e.to_string())?,
        source_kind: SourceKind::parse(&kind_raw)?,
        subject: row.try_get("subject").map_err(|e| e.to_string())?,
        cadence: Cadence::parse(&cadence_raw)?,
        label: row.try_get("label").map_err(|e| e.to_string())?,
        status: SeriesStatus::parse(&status_raw)?,
        last_collected_at: row
            .try_get("last_collected_at")
            .map_err(|e| e.to_string())?,
        last_error: row.try_get("last_error").map_err(|e| e.to_string())?,
        created_at: row.try_get("created_at").map_err(|e| e.to_string())?,
    })
}

const SELECT_COLS: &str = "id, project_id, intel_id, source_kind, subject, cadence, label, \
                           status, last_collected_at, last_error, created_at";

/// What a bind asks for. A struct rather than eight positional arguments: at
/// that width the call site stops saying which `Option<&str>` is which, and
/// `intel_id` and `label` are both `Option<&str>`.
#[derive(Debug, Clone, PartialEq)]
pub struct BindRequest<'a> {
    pub project_id: &'a str,
    pub source_kind: SourceKind,
    pub subject: &'a str,
    /// The `project_intel` row this series is about, when it is about one.
    pub intel_id: Option<&'a str>,
    /// `None` takes the source's own resolution.
    pub cadence: Option<Cadence>,
    pub label: Option<&'a str>,
}

impl<'a> BindRequest<'a> {
    /// The common case: a project, a source and a subject.
    pub fn new(project_id: &'a str, source_kind: SourceKind, subject: &'a str) -> Self {
        Self {
            project_id,
            source_kind,
            subject,
            intel_id: None,
            cadence: None,
            label: None,
        }
    }

    pub fn intel(mut self, intel_id: Option<&'a str>) -> Self {
        self.intel_id = intel_id;
        self
    }

    pub fn cadence(mut self, cadence: Option<Cadence>) -> Self {
        self.cadence = cadence;
        self
    }
}

/// Propose a series. Idempotent on `(project, source, subject)`.
///
/// Binding *proposes*; approval is the human's. The one exception is the
/// `SelfBindApprovedIntel` knob: when the series hangs off a `project_intel`
/// row a human already approved as a competitor, only the metric is new, so it
/// activates directly. That is a setting, and its default is off.
pub async fn bind(
    pool: &Pool<Sqlite>,
    knobs: &Knobs,
    req: BindRequest<'_>,
) -> Result<Series, String> {
    let BindRequest {
        project_id,
        source_kind,
        subject: subject_raw,
        intel_id,
        cadence,
        label,
    } = req;
    if project_id.trim().is_empty() {
        return Err("project is required".into());
    }
    let subject = if knobs.normalize_subjects {
        normalize_subject(source_kind, subject_raw)?
    } else {
        let s = subject_raw.trim();
        if s.is_empty() || s.chars().any(|c| c.is_control()) {
            return Err("subject is empty or contains a control character".into());
        }
        s.to_string()
    };
    let cadence = cadence.unwrap_or_else(|| source_kind.native_cadence());

    // Existing row wins: re-binding must never fork a collected history.
    if let Some(existing) = find(pool, project_id, source_kind, &subject).await? {
        return Ok(existing);
    }

    let status = match (knobs.approval, intel_id) {
        (Approval::SelfBindApprovedIntel, Some(id)) if intel_is_competitor(pool, id).await? => {
            SeriesStatus::Active
        }
        _ => SeriesStatus::Proposed,
    };
    let label = label
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| format!("{} — {}", subject, source_kind.label()));

    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO forecaster_series
         (id, project_id, intel_id, source_kind, subject, cadence, label, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(project_id)
    .bind(intel_id)
    .bind(source_kind.as_str())
    .bind(&subject)
    .bind(cadence.as_str())
    .bind(&label)
    .bind(status.as_str())
    .bind(now_iso())
    .execute(pool)
    .await
    .map_err(|e| format!("bind series: {e}"))?;

    get(pool, &id)
        .await?
        .ok_or_else(|| "series vanished after insert".to_string())
}

/// Is this intel row a competitor a human already approved? Rows only reach
/// `project_intel` through the Decision Inbox, so presence is approval.
async fn intel_is_competitor(pool: &Pool<Sqlite>, intel_id: &str) -> Result<bool, String> {
    let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM project_intel WHERE id = ?")
        .bind(intel_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("read project_intel: {e}"))?;
    Ok(kind.as_deref() == Some("competitor"))
}

pub async fn find(
    pool: &Pool<Sqlite>,
    project_id: &str,
    source_kind: SourceKind,
    subject: &str,
) -> Result<Option<Series>, String> {
    let sql = format!(
        "SELECT {SELECT_COLS} FROM forecaster_series
         WHERE project_id = ? AND source_kind = ? AND subject = ?"
    );
    // Audited: the only interpolation is the `SELECT_COLS` const above; every
    // value reaches the statement through `.bind()`.
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(project_id)
        .bind(source_kind.as_str())
        .bind(subject)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("find series: {e}"))?;
    row.as_ref().map(row_to_series).transpose()
}

pub async fn get(pool: &Pool<Sqlite>, id: &str) -> Result<Option<Series>, String> {
    let sql = format!("SELECT {SELECT_COLS} FROM forecaster_series WHERE id = ?");
    // Audited: the only interpolation is the `SELECT_COLS` const above; every
    // value reaches the statement through `.bind()`.
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("get series: {e}"))?;
    row.as_ref().map(row_to_series).transpose()
}

/// Move a proposed series to `active`. The review gate's apply step.
pub async fn set_status(
    pool: &Pool<Sqlite>,
    id: &str,
    status: SeriesStatus,
) -> Result<bool, String> {
    let res = sqlx::query("UPDATE forecaster_series SET status = ? WHERE id = ?")
        .bind(status.as_str())
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| format!("set series status: {e}"))?;
    Ok(res.rows_affected() > 0)
}

/// Every series the sweep should collect: approved, and nothing else.
pub async fn active_series(pool: &Pool<Sqlite>) -> Result<Vec<Series>, String> {
    let sql = format!(
        "SELECT {SELECT_COLS} FROM forecaster_series WHERE status = 'active' \
         ORDER BY project_id, source_kind, subject"
    );
    // Audited: the only interpolation is the `SELECT_COLS` const above; every
    // value reaches the statement through `.bind()`.
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_all(pool)
        .await
        .map_err(|e| format!("list active series: {e}"))?;
    rows.iter().map(row_to_series).collect()
}

/// Append observations. Returns how many rows were actually new.
///
/// The return value is the acceptance criterion for idempotency: a second pass
/// over the same window must return 0, not "succeeded".
pub async fn append_points(
    pool: &Pool<Sqlite>,
    series_id: &str,
    points: &[Point],
) -> Result<usize, String> {
    if points.is_empty() {
        return Ok(0);
    }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let mut inserted = 0usize;
    for p in points {
        // A non-finite value is not an observation. Storing NaN would poison
        // every downstream mean and quietly turn a forecast into garbage.
        if !p.value.is_finite() {
            continue;
        }
        let res = sqlx::query(
            "INSERT OR IGNORE INTO forecaster_points (series_id, ts, value) VALUES (?, ?, ?)",
        )
        .bind(series_id)
        .bind(&p.ts)
        .bind(p.value)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("append point: {e}"))?;
        inserted += res.rows_affected() as usize;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(inserted)
}

/// The whole series, oldest first — the order every forecasting method assumes.
pub async fn load_points(pool: &Pool<Sqlite>, series_id: &str) -> Result<Vec<Point>, String> {
    let rows =
        sqlx::query("SELECT ts, value FROM forecaster_points WHERE series_id = ? ORDER BY ts")
            .bind(series_id)
            .fetch_all(pool)
            .await
            .map_err(|e| format!("load points: {e}"))?;
    rows.iter()
        .map(|r| {
            Ok(Point {
                ts: r.try_get("ts").map_err(|e: sqlx::Error| e.to_string())?,
                value: r.try_get("value").map_err(|e: sqlx::Error| e.to_string())?,
            })
        })
        .collect()
}

/// Record that a collector ran. `error` is stored rather than logged and
/// dropped, because a silently dead collector reads exactly like a flat market.
pub async fn mark_collected(
    pool: &Pool<Sqlite>,
    series_id: &str,
    error: Option<&str>,
) -> Result<(), String> {
    sqlx::query("UPDATE forecaster_series SET last_collected_at = ?, last_error = ? WHERE id = ?")
        .bind(now_iso())
        .bind(error)
        .bind(series_id)
        .execute(pool)
        .await
        .map_err(|e| format!("mark collected: {e}"))?;
    Ok(())
}

/// The honesty surface: every series with its real counts and its verdict.
pub async fn summarize(
    pool: &Pool<Sqlite>,
    project_id: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<SeriesSummary>, String> {
    let base = format!("SELECT {SELECT_COLS} FROM forecaster_series");
    // Audited: both arms interpolate only `base` — itself just the `SELECT_COLS`
    // const above — and `project_id` reaches the statement through `.bind()`.
    let rows = match project_id {
        Some(p) => {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "{base} WHERE project_id = ? ORDER BY source_kind, subject"
            )))
            .bind(p)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "{base} ORDER BY project_id, source_kind, subject"
            )))
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| format!("summarize series: {e}"))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let s = row_to_series(row)?;
        let stats = sqlx::query(
            "SELECT COUNT(*) AS n, MIN(ts) AS first_ts, MAX(ts) AS last_ts
             FROM forecaster_points WHERE series_id = ?",
        )
        .bind(&s.id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("count points: {e}"))?;
        let points: i64 = stats.try_get("n").map_err(|e| e.to_string())?;
        let first_ts: Option<String> = stats.try_get("first_ts").map_err(|e| e.to_string())?;
        let last_ts: Option<String> = stats.try_get("last_ts").map_err(|e| e.to_string())?;
        let span_days = match (first_ts.as_deref(), last_ts.as_deref()) {
            (Some(a), Some(b)) => day_span(a, b),
            _ => 0,
        };
        let points = points.max(0) as usize;
        out.push(SeriesSummary {
            verdict: verdict_for(&s, points, now),
            series_id: s.id.clone(),
            project_id: s.project_id.clone(),
            intel_id: s.intel_id.clone(),
            source_kind: s.source_kind,
            source_label: s.source_kind.label().to_string(),
            subject_group: super::series::subject_group(s.source_kind, &s.subject)
                .map(str::to_string),
            subject: s.subject.clone(),
            cadence: s.cadence,
            label: s.label.clone(),
            status: s.status,
            points,
            span_days,
            first_ts,
            last_ts,
            last_collected_at: s.last_collected_at.clone(),
            last_error: s.last_error.clone(),
            snapshot_only: !s.source_kind.backfills(),
            official_source: s.source_kind.is_official(),
        });
    }
    Ok(out)
}

fn day_span(a: &str, b: &str) -> i64 {
    let parse =
        |s: &str| chrono::NaiveDate::parse_from_str(s.get(..10).unwrap_or(s), "%Y-%m-%d").ok();
    match (parse(a), parse(b)) {
        (Some(x), Some(y)) => (y - x).num_days(),
        _ => 0,
    }
}

/// Guard order, and the order is the argument.
///
/// Not approved beats everything — an unapproved series is not collecting, so
/// any point count it has is an artefact. A stale collector beats a long
/// history, because forecasting three-week-old numbers as if they were current
/// is a worse answer than refusing. Length is checked last, and only then can a
/// series be `Forecastable`.
pub fn verdict_for(series: &Series, points: usize, now: chrono::DateTime<chrono::Utc>) -> Verdict {
    if series.status != SeriesStatus::Active {
        return Verdict::NotBound;
    }
    let stale_after_days = match series.cadence {
        Cadence::Daily => 3,
        Cadence::Weekly => 21,
    };
    let fresh = series
        .last_collected_at
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| (now - t.with_timezone(&chrono::Utc)).num_days() <= stale_after_days)
        .unwrap_or(false);
    if !fresh {
        return Verdict::CollectorStale {
            last_collected_at: series.last_collected_at.clone(),
        };
    }
    let needed = series.cadence.min_points();
    if points < needed {
        return Verdict::InsufficientHistory { points, needed };
    }
    Verdict::Forecastable
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema::init_spectral_db;

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    fn knobs() -> Knobs {
        Knobs::default()
    }

    #[tokio::test]
    async fn binding_the_same_subject_twice_reaches_one_series() {
        let pool = pool().await;
        let a = bind(
            &pool,
            &knobs(),
            BindRequest::new("p1", SourceKind::PyPi, "LangChain_Core"),
        )
        .await
        .unwrap();
        assert_eq!(a.subject, "langchain-core", "normalized on the way in");
        assert_eq!(a.status, SeriesStatus::Proposed, "binding proposes only");
        // A differently-spelled second bind normalizes onto the same subject
        // and must reach the same row rather than fork its history.
        let b = bind(
            &pool,
            &knobs(),
            BindRequest::new("p1", SourceKind::PyPi, "langchain.core"),
        )
        .await
        .unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(
            summarize(&pool, Some("p1"), chrono::Utc::now())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn recollecting_the_same_window_is_idempotent() {
        let pool = pool().await;
        let s = bind(
            &pool,
            &knobs(),
            BindRequest::new("p1", SourceKind::Npm, "vitest"),
        )
        .await
        .unwrap();
        let window: Vec<Point> = (1..=10)
            .map(|d| Point {
                ts: format!("2026-08-{d:02}"),
                value: d as f64,
            })
            .collect();
        assert_eq!(append_points(&pool, &s.id, &window).await.unwrap(), 10);
        // The overlapping second pass the sweep actually performs.
        assert_eq!(
            append_points(&pool, &s.id, &window).await.unwrap(),
            0,
            "a re-collected window must insert zero rows"
        );
        let mut extended = window.clone();
        extended.push(Point {
            ts: "2026-08-11".into(),
            value: 11.0,
        });
        assert_eq!(append_points(&pool, &s.id, &extended).await.unwrap(), 1);
        assert_eq!(load_points(&pool, &s.id).await.unwrap().len(), 11);
    }

    #[tokio::test]
    async fn a_non_finite_value_is_not_an_observation() {
        let pool = pool().await;
        let s = bind(
            &pool,
            &knobs(),
            BindRequest::new("p1", SourceKind::Npm, "vitest"),
        )
        .await
        .unwrap();
        let n = append_points(
            &pool,
            &s.id,
            &[
                Point {
                    ts: "2026-08-01".into(),
                    value: f64::NAN,
                },
                Point {
                    ts: "2026-08-02".into(),
                    value: 3.0,
                },
            ],
        )
        .await
        .unwrap();
        assert_eq!(n, 1);
        assert_eq!(load_points(&pool, &s.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_verdict_guards_run_in_order() {
        let pool = pool().await;
        let now = chrono::Utc::now();
        let s = bind(
            &pool,
            &knobs(),
            BindRequest::new("p1", SourceKind::Npm, "vitest"),
        )
        .await
        .unwrap();

        // 1. Not approved: no point count can make this forecastable.
        let pts: Vec<Point> = (0..200)
            .map(|d| Point {
                ts: (chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
                    + chrono::Duration::days(d))
                .to_string(),
                value: d as f64,
            })
            .collect();
        append_points(&pool, &s.id, &pts).await.unwrap();
        let summary = &summarize(&pool, Some("p1"), now).await.unwrap()[0];
        assert_eq!(summary.verdict, Verdict::NotBound);
        assert_eq!(summary.points, 200);

        // 2. Approved but never collected: stale beats a long history.
        set_status(&pool, &s.id, SeriesStatus::Active)
            .await
            .unwrap();
        let summary = &summarize(&pool, Some("p1"), now).await.unwrap()[0];
        assert!(matches!(summary.verdict, Verdict::CollectorStale { .. }));

        // 3. Fresh and long enough.
        mark_collected(&pool, &s.id, None).await.unwrap();
        let summary = &summarize(&pool, Some("p1"), now).await.unwrap()[0];
        assert_eq!(summary.verdict, Verdict::Forecastable);
        assert_eq!(summary.span_days, 199);
    }

    #[tokio::test]
    async fn a_short_series_reports_how_short_rather_than_a_number() {
        let pool = pool().await;
        let now = chrono::Utc::now();
        let s = bind(
            &pool,
            &knobs(),
            BindRequest::new("p1", SourceKind::Npm, "vitest"),
        )
        .await
        .unwrap();
        set_status(&pool, &s.id, SeriesStatus::Active)
            .await
            .unwrap();
        mark_collected(&pool, &s.id, None).await.unwrap();
        append_points(
            &pool,
            &s.id,
            &[Point {
                ts: "2026-08-01".into(),
                value: 1.0,
            }],
        )
        .await
        .unwrap();
        let summary = &summarize(&pool, Some("p1"), now).await.unwrap()[0];
        assert_eq!(
            summary.verdict,
            Verdict::InsufficientHistory {
                points: 1,
                needed: 180
            },
            "the refusal names the gap so the card can render \"1 of 180\""
        );
    }

    #[tokio::test]
    async fn a_snapshot_only_source_reports_snapshot_not_a_series() {
        let pool = pool().await;
        let s = bind(
            &pool,
            &knobs(),
            BindRequest::new("p1", SourceKind::GithubRepo, "ollama/ollama"),
        )
        .await
        .unwrap();
        assert_eq!(s.subject, "ollama/ollama");
        let summary = &summarize(&pool, Some("p1"), chrono::Utc::now())
            .await
            .unwrap()[0];
        assert!(summary.snapshot_only, "GitHub cannot hand us the past");
        assert_eq!(summary.points, 0, "and we do not invent one");
    }

    #[tokio::test]
    async fn self_binding_activates_only_an_already_approved_competitor() {
        let pool = pool().await;
        sqlx::query(
            "INSERT INTO project_intel (id, project_id, kind, name, source_url, created_at)
             VALUES ('i1','p1','competitor','Rival','https://rival.example','now'),
                    ('i2','p1','adjacent','Neighbour','https://n.example','now')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let k = Knobs {
            approval: Approval::SelfBindApprovedIntel,
            ..Knobs::default()
        };
        let approved = bind(
            &pool,
            &k,
            BindRequest::new("p1", SourceKind::Npm, "rival").intel(Some("i1")),
        )
        .await
        .unwrap();
        assert_eq!(approved.status, SeriesStatus::Active);
        // An adjacent row is not an approved competitor; still proposed.
        let adjacent = bind(
            &pool,
            &k,
            BindRequest::new("p1", SourceKind::Npm, "neighbour").intel(Some("i2")),
        )
        .await
        .unwrap();
        assert_eq!(adjacent.status, SeriesStatus::Proposed);
        // And with the default knob, even an approved competitor proposes.
        let gated = bind(
            &pool,
            &knobs(),
            BindRequest::new("p1", SourceKind::Crates, "rival").intel(Some("i1")),
        )
        .await
        .unwrap();
        assert_eq!(gated.status, SeriesStatus::Proposed);
        assert_eq!(active_series(&pool).await.unwrap().len(), 1);
    }
}
