//! Collectors — turning a bound subject into a history of numbers.
//!
//! Every collector is split in three so the network is never in a test:
//!
//! * [`plan`] turns a validated `(SourceKind, subject, window)` into a list of
//!   HTTP requests. Pure. This is also the only place a URL is ever built, and
//!   it is reachable only from a parsed [`SourceKind`] and a normalized
//!   subject — so "an unknown source" cannot become a request.
//! * [`parse_response`] turns one response body into points. Pure.
//! * [`collect`] runs a [`Fetcher`] over the plan and concatenates the result,
//!   honouring each source's politeness interval.
//!
//! Tests drive [`plan`]/[`parse_response`] directly, and [`collect`] through a
//! fixture fetcher holding recorded bodies. Nothing in this module's test suite
//! opens a socket.
//!
//! **Backfill first, then increment.** A collector's whole value is the past it
//! can hand over on the day it is bound; the incremental pass afterwards is
//! cheap because re-collection is a no-op (`store::append_points`).

use async_trait::async_trait;
use chrono::{Datelike, Duration, NaiveDate, Utc};
use std::collections::HashMap;

use super::series::SourceKind;
use super::store::Point;
use super::Cadence;

/// crates.io's crawler policy (RFC 3463) requires a user agent that identifies
/// the client and carries a contact. Sending a browser string there would be
/// both rude and a policy violation, so this is not the Yahoo agent.
pub const USER_AGENT: &str =
    "permagent-forecaster/1.0 (+https://github.com/permagent/permagent-runtime)";

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// How far back a bind reaches on its first pass. Wikimedia goes to 2015 and
/// HN to 2006; three years is enough for a weekly seasonal period (52) with
/// room for folds, and it keeps a bind from making hundreds of requests.
pub const MAX_BACKFILL_DAYS: i64 = 3 * 365;

/// npm's range endpoint refuses windows longer than 18 months, and only holds
/// that much history anyway.
const NPM_MAX_RANGE_DAYS: i64 = 540;

#[derive(Debug, Clone, PartialEq)]
pub enum CollectError {
    /// The source has no collector yet. Named honestly rather than returning
    /// an empty series, which would read as "the market is flat".
    NotCollectable { kind: SourceKind, reason: String },
    /// The subject or window could not become a request.
    BadRequest(String),
    /// The network or the source failed.
    Unreachable(String),
    /// The source answered, in a shape we do not recognise.
    Malformed(String),
}

impl std::fmt::Display for CollectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCollectable { kind, reason } => {
                write!(f, "{} has no collector: {reason}", kind.label())
            }
            Self::BadRequest(m) => write!(f, "bad request: {m}"),
            Self::Unreachable(m) => write!(f, "source unreachable: {m}"),
            Self::Malformed(m) => write!(f, "source answered something unreadable: {m}"),
        }
    }
}

/// One HTTP GET in a collection plan.
///
/// `bucket_ts` is `Some` for the count sources (HN, arXiv) where one request
/// *is* one point — a search that returns only `nbHits` for a date window. It
/// is `None` where the response carries its own timestamps.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectRequest {
    pub url: String,
    pub bucket_ts: Option<String>,
}

/// Anything that can turn a URL into a body. The seam that keeps the network
/// out of the tests.
#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn get(&self, url: &str) -> Result<String, CollectError>;
}

/// The real one.
pub struct HttpFetcher {
    client: reqwest::Client,
}

impl HttpFetcher {
    pub fn new() -> Result<Self, CollectError> {
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| CollectError::Unreachable(e.to_string()))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl Fetcher for HttpFetcher {
    async fn get(&self, url: &str) -> Result<String, CollectError> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| CollectError::Unreachable(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| CollectError::Unreachable(e.to_string()))?;
        if !status.is_success() {
            // A 404 on a package name is a *user* error, not an outage, and the
            // two have different fixes. Say which.
            return Err(if status == reqwest::StatusCode::NOT_FOUND {
                CollectError::BadRequest("the source has no such subject (404)".into())
            } else {
                CollectError::Unreachable(format!("HTTP {status}"))
            });
        }
        Ok(body)
    }
}

/// A fetcher over recorded bodies. Public so the daemon's tests can use it too.
pub struct FixtureFetcher {
    pub bodies: HashMap<String, String>,
}

impl FixtureFetcher {
    pub fn new(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            bodies: pairs.into_iter().collect(),
        }
    }
}

#[async_trait]
impl Fetcher for FixtureFetcher {
    async fn get(&self, url: &str) -> Result<String, CollectError> {
        self.bodies
            .get(url)
            .cloned()
            .ok_or_else(|| CollectError::Unreachable(format!("no fixture for {url}")))
    }
}

/// Minimum gap between two requests to the same source, from its published
/// policy. crates.io asks for 1 req/s (RFC 3463); arXiv's ToS says 1 req/3 s.
/// These shape the sweep's pacing, not its price — every source here is free.
pub fn min_request_interval(kind: SourceKind) -> std::time::Duration {
    match kind {
        SourceKind::Crates => std::time::Duration::from_millis(1000),
        SourceKind::ArxivCount => std::time::Duration::from_millis(3000),
        _ => std::time::Duration::from_millis(200),
    }
}

fn pct(s: &str) -> String {
    // Percent-encode everything outside the unreserved set. The subject has
    // already been proven to be a name by `normalize_subject`; this is the
    // second layer, not the first.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Monday of the week containing `d`. Weekly buckets are Monday-anchored so a
/// series collected on different weekdays lands on the same timestamps.
fn week_start(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

/// Build the requests that collect `[since, until]` for this subject.
///
/// `subject` must already have passed `series::normalize_subject`.
pub fn plan(
    kind: SourceKind,
    subject: &str,
    cadence: Cadence,
    since: NaiveDate,
    until: NaiveDate,
) -> Result<Vec<CollectRequest>, CollectError> {
    if until < since {
        return Err(CollectError::BadRequest(
            "window ends before it starts".into(),
        ));
    }
    let s = pct(subject);
    match kind {
        SourceKind::Npm => {
            // npm's range endpoint refuses a window longer than 18 months AND
            // holds no more history than that, so asking for three years is not
            // a bigger answer — it is an error response, which would fail the
            // whole pass. Clamp, and let the registry's point count say plainly
            // that an npm-only series sits below the 180-point gate for months.
            let start = since.max(until - Duration::days(NPM_MAX_RANGE_DAYS - 1));
            Ok(vec![CollectRequest {
                url: format!(
                    "https://api.npmjs.org/downloads/range/{}:{}/{}",
                    start, until, subject
                ),
                bucket_ts: None,
            }])
        }
        SourceKind::Crates => Ok(vec![CollectRequest {
            // The downloads endpoint returns the last ~90 days of per-version
            // rows plus `meta.extra_downloads` for versions since dropped. It
            // takes no window, so the window is applied on the way out.
            url: format!("https://crates.io/api/v1/crates/{s}/downloads"),
            bucket_ts: None,
        }]),
        SourceKind::WikiPageviews => Ok(vec![CollectRequest {
            url: format!(
                "https://wikimedia.org/api/rest_v1/metrics/pageviews/per-article/\
                 en.wikipedia/all-access/user/{}/daily/{}/{}",
                s,
                since.format("%Y%m%d"),
                until.format("%Y%m%d")
            ),
            bucket_ts: None,
        }]),
        SourceKind::HnMentions => {
            // Algolia returns a count, not a history, so one request per
            // bucket. Weekly only: daily HN mention counts for a niche term are
            // mostly zeros, and a forecast of mostly-zeros is theatre.
            if cadence != Cadence::Weekly {
                return Err(CollectError::BadRequest(
                    "Hacker News mentions are collected weekly; daily counts are too sparse to \
                     forecast"
                        .into(),
                ));
            }
            let mut out = Vec::new();
            let mut bucket = week_start(since);
            while bucket <= until {
                let next = bucket + Duration::days(7);
                let from = bucket.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
                let to = next.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
                out.push(CollectRequest {
                    url: format!(
                        "https://hn.algolia.com/api/v1/search?query={s}&hitsPerPage=0\
                         &numericFilters=created_at_i%3E%3D{from},created_at_i%3C{to}"
                    ),
                    bucket_ts: Some(bucket.to_string()),
                });
                bucket = next;
            }
            Ok(out)
        }
        SourceKind::PyPi => Err(CollectError::NotCollectable {
            kind,
            reason: "pypistats holds only 180 days and the BigQuery backfill needs a credential \
                     this install does not have yet"
                .into(),
        }),
        SourceKind::ArxivCount => Err(CollectError::NotCollectable {
            kind,
            reason: "the arXiv collector is not built yet".into(),
        }),
        SourceKind::StackExchangeTag => Err(CollectError::NotCollectable {
            kind,
            reason: "Stack Exchange needs a free API key that is not configured".into(),
        }),
        SourceKind::GithubRepo => Err(CollectError::NotCollectable {
            kind,
            reason: "stargazers-with-timestamps was restricted to repo admins on 2026-06-30, so \
                     GitHub is snapshot-only: one point per sweep, no past"
                .into(),
        }),
        SourceKind::EquityClose => Err(CollectError::NotCollectable {
            kind,
            reason: "equity closes are read at request time through the Financier and are not \
                     persisted (forecaster_persist_equity_closes is off)"
                .into(),
        }),
    }
}

/// Read one response into points. Every field is read defensively: a source
/// that changes shape yields a thinner answer or an honest `Malformed`, never a
/// wrong number.
pub fn parse_response(
    kind: SourceKind,
    req: &CollectRequest,
    body: &str,
) -> Result<Vec<Point>, CollectError> {
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|e| CollectError::Malformed(e.to_string()))?;
    match kind {
        SourceKind::Npm => {
            let rows = json
                .get("downloads")
                .and_then(|d| d.as_array())
                .ok_or_else(|| CollectError::Malformed("npm: no downloads array".into()))?;
            Ok(rows
                .iter()
                .filter_map(|r| {
                    Some(Point {
                        ts: r.get("day")?.as_str()?.to_string(),
                        value: r.get("downloads")?.as_f64()?,
                    })
                })
                .collect())
        }
        SourceKind::Crates => {
            // Downloads are split across versions plus an `extra_downloads`
            // bucket for versions since yanked. Summing them is the only way to
            // get the crate's real daily total.
            let mut by_day: std::collections::BTreeMap<String, f64> = Default::default();
            let mut seen_any = false;
            for key in ["version_downloads"] {
                if let Some(rows) = json.get(key).and_then(|v| v.as_array()) {
                    seen_any = true;
                    for r in rows {
                        if let (Some(d), Some(n)) = (
                            r.get("date").and_then(|v| v.as_str()),
                            r.get("downloads").and_then(|v| v.as_f64()),
                        ) {
                            *by_day.entry(d.to_string()).or_default() += n;
                        }
                    }
                }
            }
            if let Some(rows) = json
                .get("meta")
                .and_then(|m| m.get("extra_downloads"))
                .and_then(|v| v.as_array())
            {
                seen_any = true;
                for r in rows {
                    if let (Some(d), Some(n)) = (
                        r.get("date").and_then(|v| v.as_str()),
                        r.get("downloads").and_then(|v| v.as_f64()),
                    ) {
                        *by_day.entry(d.to_string()).or_default() += n;
                    }
                }
            }
            if !seen_any {
                return Err(CollectError::Malformed(
                    "crates.io: neither version_downloads nor meta.extra_downloads".into(),
                ));
            }
            Ok(by_day
                .into_iter()
                .map(|(ts, value)| Point { ts, value })
                .collect())
        }
        SourceKind::WikiPageviews => {
            let items = json
                .get("items")
                .and_then(|i| i.as_array())
                .ok_or_else(|| CollectError::Malformed("wikimedia: no items array".into()))?;
            Ok(items
                .iter()
                .filter_map(|i| {
                    // Timestamps arrive as YYYYMMDD00. Read them as characters
                    // rather than byte ranges: a source that changes shape must
                    // yield nothing, never a panic mid-sweep.
                    let raw = i.get("timestamp")?.as_str()?;
                    let digits: Vec<char> = raw.chars().take(8).collect();
                    if digits.len() != 8 || !digits.iter().all(char::is_ascii_digit) {
                        return None;
                    }
                    let part = |r: std::ops::Range<usize>| -> String { digits[r].iter().collect() };
                    Some(Point {
                        ts: format!("{}-{}-{}", part(0..4), part(4..6), part(6..8)),
                        value: i.get("views")?.as_f64()?,
                    })
                })
                .collect())
        }
        SourceKind::HnMentions => {
            let ts = req
                .bucket_ts
                .clone()
                .ok_or_else(|| CollectError::Malformed("hn: request carried no bucket".into()))?;
            let n = json
                .get("nbHits")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| CollectError::Malformed("hn: no nbHits".into()))?;
            Ok(vec![Point { ts, value: n }])
        }
        other => Err(CollectError::NotCollectable {
            kind: other,
            reason: "no parser".into(),
        }),
    }
}

/// The result of one collection pass, with enough detail to explain itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Collected {
    pub points: Vec<Point>,
    pub requests: usize,
}

/// Run a plan. `since` defaults to the backfill horizon on a first pass and to
/// just after the last stored point on later ones — the caller decides, because
/// only it knows what is already stored.
pub async fn collect(
    fetcher: &dyn Fetcher,
    kind: SourceKind,
    subject: &str,
    cadence: Cadence,
    since: NaiveDate,
    until: NaiveDate,
) -> Result<Collected, CollectError> {
    let requests = plan(kind, subject, cadence, since, until)?;
    let gap = min_request_interval(kind);
    let mut points = Vec::new();
    for (i, req) in requests.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(gap).await;
        }
        let body = fetcher.get(&req.url).await?;
        points.extend(parse_response(kind, req, &body)?);
    }
    // Trim to the window. crates.io in particular ignores the window entirely,
    // and a point outside it would silently widen the series.
    let lo = since.to_string();
    let hi = until.to_string();
    points.retain(|p| p.ts.as_str() >= lo.as_str() && p.ts.as_str() <= hi.as_str());
    points.sort_by(|a, b| a.ts.cmp(&b.ts));
    points.dedup_by(|a, b| a.ts == b.ts);
    Ok(Collected {
        points,
        requests: requests.len(),
    })
}

/// The window a first collection should ask for.
pub fn backfill_window(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    (today - Duration::days(MAX_BACKFILL_DAYS), today)
}

/// The window an incremental collection should ask for: from the day after the
/// last stored point, with a few days of overlap so a source that revises
/// yesterday's number is picked up. Overlap is free — re-collection inserts
/// nothing.
pub fn incremental_window(last_ts: Option<&str>, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    match last_ts.and_then(|t| NaiveDate::parse_from_str(t.get(..10).unwrap_or(t), "%Y-%m-%d").ok())
    {
        Some(last) => (
            (last - Duration::days(3)).max(today - Duration::days(MAX_BACKFILL_DAYS)),
            today,
        ),
        None => backfill_window(today),
    }
}

/// `Utc::now()` as a date, in one place so tests can avoid it.
pub fn today() -> NaiveDate {
    Utc::now().date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPM_BODY: &str = r#"{"start":"2026-08-01","end":"2026-08-04","package":"vitest",
        "downloads":[{"downloads":1200,"day":"2026-08-01"},{"downloads":1310,"day":"2026-08-02"},
        {"downloads":990,"day":"2026-08-03"},{"downloads":1405,"day":"2026-08-04"}]}"#;

    const CRATES_BODY: &str = r#"{"version_downloads":[
        {"date":"2026-08-01","downloads":100,"version":1},
        {"date":"2026-08-01","downloads":25,"version":2},
        {"date":"2026-08-02","downloads":140,"version":2}],
        "meta":{"extra_downloads":[{"date":"2026-08-01","downloads":7}]}}"#;

    const WIKI_BODY: &str = r#"{"items":[
        {"project":"en.wikipedia","article":"Coworking","granularity":"daily",
         "timestamp":"2026080100","access":"all-access","agent":"user","views":812},
        {"project":"en.wikipedia","article":"Coworking","granularity":"daily",
         "timestamp":"2026080200","access":"all-access","agent":"user","views":765}]}"#;

    const HN_BODY: &str = r#"{"hits":[],"nbHits":37,"page":0,"nbPages":0,"hitsPerPage":0}"#;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn an_unknown_source_kind_is_rejected_before_it_becomes_a_url() {
        // The only way into `plan` is a parsed SourceKind, and parse refuses
        // anything outside the closed set — so the invented source never
        // reaches a URL builder at all.
        assert!(SourceKind::parse("google_trends").is_err());
        // And a source inside the set but without a collector says so by name
        // rather than returning an empty series.
        let err = plan(
            SourceKind::GithubRepo,
            "ollama/ollama",
            Cadence::Daily,
            d("2026-01-01"),
            d("2026-08-01"),
        )
        .unwrap_err();
        match err {
            CollectError::NotCollectable { kind, reason } => {
                assert_eq!(kind, SourceKind::GithubRepo);
                assert!(reason.contains("snapshot-only"), "{reason}");
            }
            other => panic!("expected NotCollectable, got {other:?}"),
        }
    }

    /// npm holds 18 months and refuses a longer range outright. Asking for
    /// three years would be an error response, not a bigger answer.
    #[test]
    fn npm_clamps_a_long_window_to_the_eighteen_months_it_actually_has() {
        let reqs = plan(
            SourceKind::Npm,
            "vitest",
            Cadence::Daily,
            d("2023-01-01"),
            d("2026-08-01"),
        )
        .unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(
            reqs[0].url,
            "https://api.npmjs.org/downloads/range/2025-02-08:2026-08-01/vitest"
        );
        // A window already inside the horizon is passed through untouched.
        let reqs = plan(
            SourceKind::Npm,
            "vitest",
            Cadence::Daily,
            d("2026-07-01"),
            d("2026-08-01"),
        )
        .unwrap();
        assert_eq!(
            reqs[0].url,
            "https://api.npmjs.org/downloads/range/2026-07-01:2026-08-01/vitest"
        );
    }

    #[test]
    fn a_subject_is_percent_encoded_on_the_way_into_a_url() {
        let reqs = plan(
            SourceKind::WikiPageviews,
            "Large_language_model",
            Cadence::Daily,
            d("2026-08-01"),
            d("2026-08-02"),
        )
        .unwrap();
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0]
            .url
            .contains("/Large_language_model/daily/20260801/20260802"));
        // A subject with a slash cannot walk out of its path segment.
        let reqs = plan(
            SourceKind::WikiPageviews,
            "A/B_testing",
            Cadence::Daily,
            d("2026-08-01"),
            d("2026-08-02"),
        )
        .unwrap();
        assert!(reqs[0].url.contains("A%2FB_testing"), "{}", reqs[0].url);
    }

    #[test]
    fn hn_buckets_one_request_per_week_and_refuses_daily() {
        let reqs = plan(
            SourceKind::HnMentions,
            "agent memory",
            Cadence::Weekly,
            d("2026-08-03"),
            d("2026-08-24"),
        )
        .unwrap();
        assert_eq!(reqs.len(), 4);
        assert_eq!(reqs[0].bucket_ts.as_deref(), Some("2026-08-03"));
        assert!(
            reqs[0].url.contains("query=agent%20memory"),
            "{}",
            reqs[0].url
        );
        assert!(reqs[0].url.contains("created_at_i"));
        let err = plan(
            SourceKind::HnMentions,
            "x",
            Cadence::Daily,
            d("2026-08-01"),
            d("2026-08-02"),
        )
        .unwrap_err();
        assert!(matches!(err, CollectError::BadRequest(_)));
    }

    #[test]
    fn each_source_parses_its_recorded_body() {
        let req = CollectRequest {
            url: String::new(),
            bucket_ts: None,
        };
        let npm = parse_response(SourceKind::Npm, &req, NPM_BODY).unwrap();
        assert_eq!(npm.len(), 4);
        assert_eq!(
            npm[0],
            Point {
                ts: "2026-08-01".into(),
                value: 1200.0
            }
        );

        // crates.io splits a day across versions; the crate's number is the sum.
        let crates = parse_response(SourceKind::Crates, &req, CRATES_BODY).unwrap();
        assert_eq!(crates.len(), 2);
        assert_eq!(
            crates[0],
            Point {
                ts: "2026-08-01".into(),
                value: 132.0
            }
        );
        assert_eq!(
            crates[1],
            Point {
                ts: "2026-08-02".into(),
                value: 140.0
            }
        );

        let wiki = parse_response(SourceKind::WikiPageviews, &req, WIKI_BODY).unwrap();
        assert_eq!(wiki.len(), 2);
        assert_eq!(
            wiki[0],
            Point {
                ts: "2026-08-01".into(),
                value: 812.0
            }
        );

        let hn_req = CollectRequest {
            url: String::new(),
            bucket_ts: Some("2026-08-03".into()),
        };
        let hn = parse_response(SourceKind::HnMentions, &hn_req, HN_BODY).unwrap();
        assert_eq!(
            hn,
            vec![Point {
                ts: "2026-08-03".into(),
                value: 37.0
            }]
        );
    }

    #[test]
    fn a_source_that_changed_shape_is_malformed_not_empty() {
        let req = CollectRequest {
            url: String::new(),
            bucket_ts: None,
        };
        assert!(matches!(
            parse_response(SourceKind::Npm, &req, r#"{"error":"not found"}"#),
            Err(CollectError::Malformed(_))
        ));
        assert!(matches!(
            parse_response(SourceKind::Crates, &req, r#"{"crate":{}}"#),
            Err(CollectError::Malformed(_))
        ));
        assert!(matches!(
            parse_response(SourceKind::WikiPageviews, &req, "not json"),
            Err(CollectError::Malformed(_))
        ));
    }

    #[tokio::test]
    async fn binding_a_series_backfills_before_it_starts_accumulating() {
        // One recorded body, no socket. The point of the test is that the very
        // first pass returns history, not a single point from today.
        let (since, until) = (d("2026-08-01"), d("2026-08-04"));
        let url = plan(SourceKind::Npm, "vitest", Cadence::Daily, since, until).unwrap()[0]
            .url
            .clone();
        let fetcher = FixtureFetcher::new([(url, NPM_BODY.to_string())]);
        let got = collect(
            &fetcher,
            SourceKind::Npm,
            "vitest",
            Cadence::Daily,
            since,
            until,
        )
        .await
        .unwrap();
        assert_eq!(got.requests, 1);
        assert_eq!(got.points.len(), 4, "a first pass hands over the past");
        assert_eq!(got.points[0].ts, "2026-08-01");
        assert_eq!(got.points[3].ts, "2026-08-04");
    }

    #[tokio::test]
    async fn points_outside_the_window_are_trimmed_not_stored() {
        // crates.io ignores the window entirely and always returns ~90 days.
        let (since, until) = (d("2026-08-02"), d("2026-08-02"));
        let url = plan(SourceKind::Crates, "serde", Cadence::Daily, since, until).unwrap()[0]
            .url
            .clone();
        let fetcher = FixtureFetcher::new([(url, CRATES_BODY.to_string())]);
        let got = collect(
            &fetcher,
            SourceKind::Crates,
            "serde",
            Cadence::Daily,
            since,
            until,
        )
        .await
        .unwrap();
        assert_eq!(
            got.points,
            vec![Point {
                ts: "2026-08-02".into(),
                value: 140.0
            }]
        );
    }

    #[tokio::test]
    async fn an_unreachable_source_is_an_error_not_an_empty_series() {
        let fetcher = FixtureFetcher::new([]);
        let err = collect(
            &fetcher,
            SourceKind::Npm,
            "vitest",
            Cadence::Daily,
            d("2026-08-01"),
            d("2026-08-04"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CollectError::Unreachable(_)));
    }

    #[test]
    fn the_incremental_window_overlaps_the_last_stored_point() {
        let (since, until) = incremental_window(Some("2026-08-20"), d("2026-08-24"));
        assert_eq!(
            since,
            d("2026-08-17"),
            "three days of overlap catches revisions"
        );
        assert_eq!(until, d("2026-08-24"));
        // No history at all means the full backfill horizon.
        let (since, _) = incremental_window(None, d("2026-08-24"));
        assert_eq!(since, d("2026-08-24") - Duration::days(MAX_BACKFILL_DAYS));
    }

    #[test]
    fn politeness_intervals_match_the_published_policies() {
        assert_eq!(min_request_interval(SourceKind::Crates).as_millis(), 1000);
        assert_eq!(
            min_request_interval(SourceKind::ArxivCount).as_millis(),
            3000
        );
        assert!(
            USER_AGENT.contains("permagent"),
            "crates.io requires a contact UA"
        );
    }
}
