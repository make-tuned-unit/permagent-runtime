//! Grow analytics — provider-pluggable, READ-ONLY web-analytics API client.
//!
//! Ruled decision (2026-07-20): the Grow tab's analytics lens connects to an
//! **existing** analytics account via its stats API — we are a client, not a
//! collector. No events pipeline, no self-hosted PostHog bridge (this
//! supersedes the PostHog plan the epic sketched). Providers:
//!
//! - **Plausible (v1 Stats API)** — `GET /api/v1/stats/aggregate`. The v1 API
//!   is what Plausible Community Edition ships, so this variant covers
//!   self-hosted CE and still works against cloud today.
//! - **Plausible Cloud (v2)** — `POST /api/v2/query`, the current cloud API.
//! - **GoatCounter** — `GET /api/v0/stats/total`. The site lives in the host
//!   (`https://mysite.goatcounter.com`), so no separate site id is needed.
//!
//! Everything here is a read: we fetch visitors/pageviews over a period and
//! map them into the shapes the Grow funnel/tiles render. The connection
//! config (provider, base URL, site id) lives in the project's
//! `metadata_json` bag under [`METADATA_KEY`]; the API key lives in the
//! secret store under [`api_key_secret_key`] — never in the DB, never echoed
//! back to the UI.
//!
//! ## Sovereignty
//!
//! Any fetch here is cloud egress. The egress *audit log* currently
//! intercepts LLM provider calls only (`SovereignGuardProvider`), so this
//! module does not flow through an audited path — instead it fails closed:
//! when global sovereign mode is on, [`fetch_stats`] refuses before opening a
//! connection. Routing non-LLM HTTP through the egress audit log is a flagged
//! follow-up (see the PR / sovereignty-router direction).

use permagent::sovereignty;
use serde::{Deserialize, Serialize};

/// Key inside `projects.metadata_json` holding the [`AnalyticsConnection`].
pub const METADATA_KEY: &str = "analytics";

/// Secret-store key for a project's analytics API key. Namespaced per project
/// so multiple projects can point at different accounts.
pub fn api_key_secret_key(project_id: &str) -> String {
    format!("grow_analytics_api_key_{project_id}")
}

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalyticsProvider {
    /// Plausible v1 Stats API (Community-Edition compatible).
    Plausible,
    /// Plausible v2 query API (cloud).
    PlausibleV2,
    /// GoatCounter v0 API.
    Goatcounter,
}

impl AnalyticsProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            AnalyticsProvider::Plausible => "plausible",
            AnalyticsProvider::PlausibleV2 => "plausible_v2",
            AnalyticsProvider::Goatcounter => "goatcounter",
        }
    }
}

/// Per-project connection config — the non-secret part, stored in the
/// project's `metadata_json` under [`METADATA_KEY`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsConnection {
    pub provider: AnalyticsProvider,
    /// Instance base URL, no trailing slash (e.g. `https://plausible.io`,
    /// `https://stats.example.com`, `https://mysite.goatcounter.com`).
    pub base_url: String,
    /// Plausible `site_id` (the site's domain). Unused by GoatCounter, where
    /// the site is the host itself.
    #[serde(default)]
    pub site_id: String,
}

/// Read the connection out of a project's metadata bag. A missing or
/// malformed entry reads as "not connected" (the honest empty state).
pub fn connection_from_metadata(metadata: &serde_json::Value) -> Option<AnalyticsConnection> {
    let raw = metadata.get(METADATA_KEY)?;
    match serde_json::from_value(raw.clone()) {
        Ok(conn) => Some(conn),
        Err(e) => {
            tracing::warn!("malformed analytics connection in project metadata: {e}");
            None
        }
    }
}

/// Fetch window. Fixed variants (not free-form days) because Plausible v1
/// only accepts a fixed set of period strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsPeriod {
    Days7,
    Days30,
}

impl StatsPeriod {
    pub fn days(self) -> u32 {
        match self {
            StatsPeriod::Days7 => 7,
            StatsPeriod::Days30 => 30,
        }
    }
    /// The literal Plausible period / date_range string ("7d" / "30d" are
    /// valid in both v1 `period` and v2 `date_range`).
    fn plausible_str(self) -> &'static str {
        match self {
            StatsPeriod::Days7 => "7d",
            StatsPeriod::Days30 => "30d",
        }
    }
    /// Parse the wire form used by the stats route query param.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "7d" => Some(StatsPeriod::Days7),
            "30d" => Some(StatsPeriod::Days30),
            _ => None,
        }
    }
}

/// The core metrics the Grow funnel needs. `Option` per metric because not
/// every provider exposes both: GoatCounter's site-wide total is a visitor
/// count with no separate pageview aggregate — an absent metric renders as
/// the honest "—", never a faked number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsStats {
    pub visitors: Option<u64>,
    pub pageviews: Option<u64>,
    pub period_days: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum AnalyticsError {
    #[error("sovereign mode is on — cloud analytics fetch refused (no egress)")]
    SovereignBlocked,
    #[error("invalid analytics connection: {0}")]
    InvalidConfig(String),
    #[error("analytics provider returned HTTP {status}: {detail}")]
    Http { status: u16, detail: String },
    #[error("could not reach analytics provider: {0}")]
    Network(String),
    #[error("could not parse analytics response: {0}")]
    Parse(String),
}

// ── Validation ───────────────────────────────────────────────────────────────

/// Normalize a user-entered base URL: trim, strip trailing slashes, require
/// an http(s) scheme so a bare hostname fails loudly at save time instead of
/// as a confusing fetch error later.
pub fn normalize_base_url(raw: &str) -> Result<String, AnalyticsError> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AnalyticsError::InvalidConfig("base URL is required".into()));
    }
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err(AnalyticsError::InvalidConfig(
            "base URL must start with https:// or http://".into(),
        ));
    }
    Ok(trimmed.to_string())
}

/// Validate a connection is complete for its provider.
pub fn validate_connection(conn: &AnalyticsConnection) -> Result<(), AnalyticsError> {
    normalize_base_url(&conn.base_url)?;
    match conn.provider {
        AnalyticsProvider::Plausible | AnalyticsProvider::PlausibleV2 => {
            if conn.site_id.trim().is_empty() {
                return Err(AnalyticsError::InvalidConfig(
                    "site_id (the site's domain in Plausible) is required".into(),
                ));
            }
        }
        AnalyticsProvider::Goatcounter => {} // site lives in the host
    }
    Ok(())
}

// ── Response parsing (pure — unit-tested against fixture JSON) ───────────────

fn u64_at<'a>(v: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    let mut cur = v;
    for p in path {
        cur = cur.get(p)?;
    }
    Some(cur)
}

/// A metric value that some providers return as an integer and (defensively)
/// may arrive as a float. Absent or non-numeric → `None`.
fn as_count(v: Option<&serde_json::Value>) -> Option<u64> {
    let v = v?;
    v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64))
}

/// Plausible v1 `GET /api/v1/stats/aggregate?metrics=visitors,pageviews` →
/// `{"results":{"visitors":{"value":n},"pageviews":{"value":n}}}`.
pub fn parse_plausible_v1(body: &str) -> Result<(Option<u64>, Option<u64>), AnalyticsError> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| AnalyticsError::Parse(e.to_string()))?;
    let results = v
        .get("results")
        .ok_or_else(|| AnalyticsError::Parse("missing `results`".into()))?;
    let visitors = as_count(u64_at(results, &["visitors", "value"]));
    let pageviews = as_count(u64_at(results, &["pageviews", "value"]));
    if visitors.is_none() && pageviews.is_none() {
        return Err(AnalyticsError::Parse(
            "no visitors/pageviews in `results`".into(),
        ));
    }
    Ok((visitors, pageviews))
}

/// Plausible v2 `POST /api/v2/query` with `metrics:["visitors","pageviews"]`
/// and no dimensions → `{"results":[{"metrics":[v,p],"dimensions":[]}],...}`.
pub fn parse_plausible_v2(body: &str) -> Result<(Option<u64>, Option<u64>), AnalyticsError> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| AnalyticsError::Parse(e.to_string()))?;
    let row = v
        .get("results")
        .and_then(|r| r.get(0))
        .ok_or_else(|| AnalyticsError::Parse("missing `results[0]`".into()))?;
    let metrics = row
        .get("metrics")
        .and_then(|m| m.as_array())
        .ok_or_else(|| AnalyticsError::Parse("missing `results[0].metrics`".into()))?;
    // Metric order mirrors the request order: [visitors, pageviews].
    let visitors = as_count(metrics.first());
    let pageviews = as_count(metrics.get(1));
    if visitors.is_none() && pageviews.is_none() {
        return Err(AnalyticsError::Parse("empty metrics row".into()));
    }
    Ok((visitors, pageviews))
}

/// GoatCounter `GET /api/v0/stats/total` → `{"total":n,"total_events":n,
/// "total_utc":n,...}`. Per the API schema, `total` is "Total number of
/// visitors (including events)" — GoatCounter exposes no separate site-wide
/// pageview aggregate, so pageviews stays `None` (honest "—" in the UI).
pub fn parse_goatcounter_total(body: &str) -> Result<(Option<u64>, Option<u64>), AnalyticsError> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| AnalyticsError::Parse(e.to_string()))?;
    let total =
        as_count(v.get("total")).ok_or_else(|| AnalyticsError::Parse("missing `total`".into()))?;
    Ok((Some(total), None))
}

// ── Request building ─────────────────────────────────────────────────────────

/// GoatCounter wants RFC3339 start/end "rounded to the hour".
fn goatcounter_range(period: StatsPeriod, now: chrono::DateTime<chrono::Utc>) -> (String, String) {
    use chrono::Timelike;
    let end = now
        .date_naive()
        .and_hms_opt(now.hour(), 0, 0)
        .expect("hour/0/0 is always a valid time")
        .and_utc();
    let start = end - chrono::Duration::days(period.days() as i64);
    let fmt = "%Y-%m-%dT%H:%M:%SZ";
    (start.format(fmt).to_string(), end.format(fmt).to_string())
}

/// Fetch visitors/pageviews for the period from the connected provider.
///
/// Fails closed under global sovereign mode BEFORE any connection is opened —
/// analytics HTTP is cloud egress and (unlike LLM calls) does not yet flow
/// through the audited egress path.
pub async fn fetch_stats(
    client: &reqwest::Client,
    conn: &AnalyticsConnection,
    api_key: &str,
    period: StatsPeriod,
) -> Result<AnalyticsStats, AnalyticsError> {
    if sovereignty::global_sovereign_mode() {
        return Err(AnalyticsError::SovereignBlocked);
    }
    validate_connection(conn)?;
    let base = normalize_base_url(&conn.base_url)?;

    let response = match conn.provider {
        AnalyticsProvider::Plausible => {
            let url = format!(
                "{base}/api/v1/stats/aggregate?site_id={}&period={}&metrics=visitors,pageviews",
                urlencoding::encode(conn.site_id.trim()),
                period.plausible_str()
            );
            client.get(url).bearer_auth(api_key).send().await
        }
        AnalyticsProvider::PlausibleV2 => {
            let body = serde_json::json!({
                "site_id": conn.site_id.trim(),
                "metrics": ["visitors", "pageviews"],
                "date_range": period.plausible_str(),
            });
            client
                .post(format!("{base}/api/v2/query"))
                .bearer_auth(api_key)
                .json(&body)
                .send()
                .await
        }
        AnalyticsProvider::Goatcounter => {
            let (start, end) = goatcounter_range(period, chrono::Utc::now());
            let url = format!(
                "{base}/api/v0/stats/total?start={}&end={}",
                urlencoding::encode(&start),
                urlencoding::encode(&end)
            );
            client
                .get(url)
                .bearer_auth(api_key)
                .header("Content-Type", "application/json")
                .send()
                .await
        }
    }
    .map_err(|e| AnalyticsError::Network(e.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| AnalyticsError::Network(e.to_string()))?;
    if !status.is_success() {
        // Provider error bodies are small JSON; truncate defensively so an
        // HTML error page can't flood the UI/logs.
        let detail: String = body.chars().take(300).collect();
        return Err(AnalyticsError::Http {
            status: status.as_u16(),
            detail,
        });
    }

    let (visitors, pageviews) = match conn.provider {
        AnalyticsProvider::Plausible => parse_plausible_v1(&body)?,
        AnalyticsProvider::PlausibleV2 => parse_plausible_v2(&body)?,
        AnalyticsProvider::Goatcounter => parse_goatcounter_total(&body)?,
    };
    Ok(AnalyticsStats {
        visitors,
        pageviews,
        period_days: period.days(),
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(provider: AnalyticsProvider, base_url: &str) -> AnalyticsConnection {
        AnalyticsConnection {
            provider,
            base_url: base_url.to_string(),
            site_id: "example.com".to_string(),
        }
    }

    // ── fixture parsing (shapes grounded in the providers' API docs) ──

    #[test]
    fn plausible_v1_aggregate_parses() {
        // https://plausible.io/docs/stats-api-v1 aggregate example (trimmed to
        // the metrics we request).
        let body = r#"{"results":{"pageviews":{"value":673814},"visitors":{"value":201524}}}"#;
        let (v, p) = parse_plausible_v1(body).unwrap();
        assert_eq!(v, Some(201524));
        assert_eq!(p, Some(673814));
    }

    #[test]
    fn plausible_v1_missing_results_is_parse_error() {
        assert!(matches!(
            parse_plausible_v1(r#"{"error":"invalid site"}"#),
            Err(AnalyticsError::Parse(_))
        ));
        assert!(matches!(
            parse_plausible_v1("not json"),
            Err(AnalyticsError::Parse(_))
        ));
    }

    #[test]
    fn plausible_v2_query_parses_in_request_metric_order() {
        // https://plausible.io/docs/stats-api response shape for
        // metrics ["visitors","pageviews"], no dimensions.
        let body = r#"{"results":[{"metrics":[99,182],"dimensions":[]}],"meta":{},"query":{}}"#;
        let (v, p) = parse_plausible_v2(body).unwrap();
        assert_eq!(v, Some(99));
        assert_eq!(p, Some(182));
    }

    #[test]
    fn plausible_v2_empty_results_is_parse_error() {
        assert!(matches!(
            parse_plausible_v2(r#"{"results":[],"meta":{}}"#),
            Err(AnalyticsError::Parse(_))
        ));
    }

    #[test]
    fn goatcounter_total_maps_to_visitors_only() {
        // /api/v0/stats/total schema: total = "Total number of visitors
        // (including events)"; no site-wide pageview aggregate exists.
        let body = r#"{"total":5321,"total_events":12,"total_utc":5321,"stats":[]}"#;
        let (v, p) = parse_goatcounter_total(body).unwrap();
        assert_eq!(v, Some(5321));
        assert_eq!(
            p, None,
            "GoatCounter exposes no pageview aggregate — stay honest"
        );
    }

    #[test]
    fn goatcounter_missing_total_is_parse_error() {
        assert!(matches!(
            parse_goatcounter_total(r#"{"count":1}"#),
            Err(AnalyticsError::Parse(_))
        ));
    }

    // ── config validation / (de)serialization ──

    #[test]
    fn base_url_is_normalized_and_scheme_checked() {
        assert_eq!(
            normalize_base_url("  https://stats.example.com/  ").unwrap(),
            "https://stats.example.com"
        );
        assert!(matches!(
            normalize_base_url("stats.example.com"),
            Err(AnalyticsError::InvalidConfig(_))
        ));
        assert!(matches!(
            normalize_base_url("   "),
            Err(AnalyticsError::InvalidConfig(_))
        ));
    }

    #[test]
    fn plausible_requires_site_id_goatcounter_does_not() {
        let mut c = conn(AnalyticsProvider::Plausible, "https://plausible.io");
        c.site_id = String::new();
        assert!(validate_connection(&c).is_err());

        let mut g = conn(AnalyticsProvider::Goatcounter, "https://me.goatcounter.com");
        g.site_id = String::new();
        assert!(validate_connection(&g).is_ok());
    }

    #[test]
    fn provider_wire_names_are_stable() {
        // These strings are stored in project metadata — a rename is a silent
        // disconnect for existing users, so pin them.
        for (p, s) in [
            (AnalyticsProvider::Plausible, "plausible"),
            (AnalyticsProvider::PlausibleV2, "plausible_v2"),
            (AnalyticsProvider::Goatcounter, "goatcounter"),
        ] {
            assert_eq!(serde_json::to_value(p).unwrap(), serde_json::json!(s));
            assert_eq!(p.as_str(), s);
        }
    }

    #[test]
    fn connection_roundtrips_through_metadata_and_tolerates_garbage() {
        let c = conn(AnalyticsProvider::PlausibleV2, "https://plausible.io");
        let metadata = serde_json::json!({ (METADATA_KEY): c.clone() });
        assert_eq!(connection_from_metadata(&metadata), Some(c));

        // Absent / malformed → not connected, never a panic.
        assert_eq!(connection_from_metadata(&serde_json::json!({})), None);
        assert_eq!(
            connection_from_metadata(
                &serde_json::json!({ (METADATA_KEY): {"provider":"posthog"} })
            ),
            None
        );
    }

    #[test]
    fn goatcounter_range_is_hour_rounded_and_spans_period() {
        let now = "2026-07-20T15:42:31Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap();
        let (start, end) = goatcounter_range(StatsPeriod::Days30, now);
        assert_eq!(end, "2026-07-20T15:00:00Z");
        assert_eq!(start, "2026-06-20T15:00:00Z");
    }

    // ── sovereignty fail-closed ──

    #[tokio::test]
    async fn sovereign_mode_refuses_before_any_connection() {
        let _guard = env_lock::lock_env([("SOVEREIGN_MODE", Some("true"))]);
        sovereignty::invalidate_global_mode_cache();

        // base_url points at a port nothing listens on: if the guard failed
        // we'd get a Network error, not SovereignBlocked.
        let c = conn(AnalyticsProvider::Plausible, "http://127.0.0.1:1");
        let client = reqwest::Client::new();
        let err = fetch_stats(&client, &c, "key", StatsPeriod::Days7)
            .await
            .expect_err("sovereign mode must refuse");
        assert!(matches!(err, AnalyticsError::SovereignBlocked), "{err}");

        sovereignty::invalidate_global_mode_cache();
    }

    // ── live wire-shape tests against a local mock server (no real egress) ──

    mod wire {
        use super::*;
        use wiremock::matchers::{body_json, header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        /// Pin sovereign mode OFF (env override) for the duration of a wire
        /// test — the suite must not depend on ambient config.
        fn sovereign_off() -> env_lock::EnvGuard<'static> {
            let guard = env_lock::lock_env([("SOVEREIGN_MODE", Some("false"))]);
            sovereignty::invalidate_global_mode_cache();
            guard
        }

        #[tokio::test]
        async fn plausible_v1_request_and_mapping() {
            let _guard = sovereign_off();
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/stats/aggregate"))
                .and(query_param("site_id", "example.com"))
                .and(query_param("period", "30d"))
                .and(query_param("metrics", "visitors,pageviews"))
                .and(header("authorization", "Bearer test-key"))
                .respond_with(ResponseTemplate::new(200).set_body_string(
                    r#"{"results":{"visitors":{"value":42},"pageviews":{"value":99}}}"#,
                ))
                .expect(1)
                .mount(&server)
                .await;

            let c = conn(AnalyticsProvider::Plausible, &server.uri());
            let stats = fetch_stats(&reqwest::Client::new(), &c, "test-key", StatsPeriod::Days30)
                .await
                .unwrap();
            assert_eq!(stats.visitors, Some(42));
            assert_eq!(stats.pageviews, Some(99));
            assert_eq!(stats.period_days, 30);
        }

        #[tokio::test]
        async fn plausible_v2_posts_query_body() {
            let _guard = sovereign_off();
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/api/v2/query"))
                .and(header("authorization", "Bearer k2"))
                .and(body_json(serde_json::json!({
                    "site_id": "example.com",
                    "metrics": ["visitors", "pageviews"],
                    "date_range": "7d",
                })))
                .respond_with(ResponseTemplate::new(200).set_body_string(
                    r#"{"results":[{"metrics":[7,11],"dimensions":[]}],"meta":{},"query":{}}"#,
                ))
                .expect(1)
                .mount(&server)
                .await;

            let c = conn(AnalyticsProvider::PlausibleV2, &server.uri());
            let stats = fetch_stats(&reqwest::Client::new(), &c, "k2", StatsPeriod::Days7)
                .await
                .unwrap();
            assert_eq!(stats.visitors, Some(7));
            assert_eq!(stats.pageviews, Some(11));
            assert_eq!(stats.period_days, 7);
        }

        #[tokio::test]
        async fn goatcounter_hits_total_endpoint() {
            let _guard = sovereign_off();
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v0/stats/total"))
                .and(header("authorization", "Bearer gc"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_string(r#"{"total":123,"total_events":0,"total_utc":123}"#),
                )
                .expect(1)
                .mount(&server)
                .await;

            let mut c = conn(AnalyticsProvider::Goatcounter, &server.uri());
            c.site_id = String::new(); // site lives in the host for GoatCounter
            let stats = fetch_stats(&reqwest::Client::new(), &c, "gc", StatsPeriod::Days30)
                .await
                .unwrap();
            assert_eq!(stats.visitors, Some(123));
            assert_eq!(stats.pageviews, None);
        }

        #[tokio::test]
        async fn non_success_status_surfaces_as_http_error() {
            let _guard = sovereign_off();
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/stats/aggregate"))
                .respond_with(
                    ResponseTemplate::new(401).set_body_string(r#"{"error":"invalid API key"}"#),
                )
                .mount(&server)
                .await;

            let c = conn(AnalyticsProvider::Plausible, &server.uri());
            let err = fetch_stats(&reqwest::Client::new(), &c, "bad", StatsPeriod::Days30)
                .await
                .expect_err("401 must not read as success");
            match err {
                AnalyticsError::Http { status, detail } => {
                    assert_eq!(status, 401);
                    assert!(detail.contains("invalid API key"));
                }
                other => panic!("expected Http error, got {other}"),
            }
        }
    }
}
