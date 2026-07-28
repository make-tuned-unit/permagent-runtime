//! First-party web analytics (#23) — the daemon IS the collector; no
//! third-party analytics dependency. Jesse's ruling (2026-07-28) supersedes
//! the 2026-07-20 connector-only decision: the connector lens
//! (`grow_analytics.rs`) stays for people who already have a provider, and
//! this module adds the self-hosted path:
//!
//!   POST /api/projects/{id}/analytics/first_party/enable — mint the site key
//!        (idempotent), optionally set the ingest base URL; returns the full
//!        setup payload (snippet + coding-agent prompt).
//!   GET  /api/projects/{id}/analytics/first_party        — status + setup payload
//!   GET  /api/projects/{id}/analytics/first_party/stats  — ?days=30 aggregates
//!   POST /collect/{site_key}                             — the beacon endpoint
//!
//! The collect endpoint is deliberately **outside** both the bearer middleware
//! and the origin guard: the user's own website posts beacons cross-origin
//! from visitors' browsers, which can never hold a daemon token. Its exposure
//! is bounded: 128-bit random site key in the path, body parsed from raw
//! bytes (≤2 KiB), fixed field whitelist, per-key rate limit, and it can only
//! ever INSERT rows into `analytics_events`.
//!
//! Visitor uniques are privacy-preserving: sha256(site_key, UA,
//! Accept-Language, UTC day) — no IP stored, rotates daily.

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use permagent::projects::{self, Project, UpdateProject};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite};
use std::sync::{Arc, Mutex, OnceLock};

/// Metadata bag key on the project (`metadata_json.first_party_analytics`).
pub const METADATA_KEY: &str = "first_party_analytics";

/// Per-site-key rate limit: events accepted per rolling minute.
const EVENTS_PER_MINUTE: u32 = 300;
/// Beacon body cap — a legitimate beacon is < 300 bytes.
const MAX_BODY_BYTES: usize = 2048;

// ── Config stored in the project metadata bag ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirstPartyConfig {
    site_key: String,
    /// Base URL visitors' browsers can reach the daemon at (LAN URL, tailnet
    /// or tunnel hostname). The snippet embeds `<ingest_base>/collect/<key>`.
    #[serde(default)]
    ingest_base: Option<String>,
}

fn config_from_project(project: &Project) -> Option<FirstPartyConfig> {
    project
        .metadata_json
        .get(METADATA_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

// ── Wire types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnableRequest {
    #[serde(default)]
    ingest_base: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupResponse {
    enabled: bool,
    site_key: Option<String>,
    ingest_base: Option<String>,
    ingest_url: Option<String>,
    snippet: Option<String>,
    agent_prompt: Option<String>,
    /// True once at least one event has ever been ingested for this project.
    receiving: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DayCount {
    day: String,
    pageviews: i64,
    visitors: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NamedCount {
    name: String,
    count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FirstPartyStats {
    enabled: bool,
    receiving: bool,
    period_days: u32,
    pageviews: i64,
    visitors: i64,
    /// Events in the last 5 minutes — the "live" indicator.
    events_last_5m: i64,
    by_day: Vec<DayCount>,
    top_pages: Vec<NamedCount>,
    top_referrers: Vec<NamedCount>,
    top_events: Vec<NamedCount>,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn project_and_pool(
    state: &AppState,
    project_id: &str,
) -> Result<(Pool<Sqlite>, Project), StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let project = projects::get_project_by_id_or_slug(&pool, project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok((pool, project))
}

fn mint_site_key() -> String {
    let bytes: [u8; 16] = rand::random();
    hex::encode(bytes)
}

fn snippet_for(ingest_url: &str) -> String {
    // sendBeacon with a stringified body avoids a CORS preflight entirely
    // (text/plain simple request); fetch keepalive is the fallback.
    format!(
        r#"<script>
(function () {{
  var E = "{ingest_url}";
  function send(kind, name) {{
    var body = JSON.stringify({{ k: kind, p: location.pathname, r: document.referrer || null, n: name || null }});
    if (!(navigator.sendBeacon && navigator.sendBeacon(E, body))) {{
      fetch(E, {{ method: "POST", body: body, keepalive: true }}).catch(function () {{}});
    }}
  }}
  send("pv");
  var push = history.pushState;
  history.pushState = function () {{ push.apply(this, arguments); send("pv"); }};
  addEventListener("popstate", function () {{ send("pv"); }});
  window.permagent = window.permagent || {{}};
  window.permagent.event = function (name) {{ send("ev", name); }};
}})();
</script>"#
    )
}

fn agent_prompt_for(project_name: &str, ingest_url: &str, snippet: &str) -> String {
    format!(
        "Set up first-party web analytics for the site \"{project_name}\".\n\n\
         1. Add the following snippet to every page, just before </head>. For a \
         framework app (Next.js/Astro/Vite SPA), put it in the root layout or \
         index.html so it loads once on every route:\n\n{snippet}\n\n\
         2. The endpoint {ingest_url} is a self-hosted Permagent analytics \
         collector. Do not replace it with any third-party analytics service, \
         and do not add any other analytics scripts.\n\
         3. The snippet already tracks SPA route changes via history.pushState \
         and popstate. For custom conversion events, call \
         window.permagent.event(\"signup\") (or any short name) at the moment \
         the action succeeds.\n\
         4. IMPORTANT: the collector URL must be reachable from visitors' \
         browsers. If this site is deployed publicly and the URL above is a \
         LAN/localhost address, stop and report that the ingest base URL needs \
         to be a tunnel or tailnet hostname instead — do not guess one.\n\
         5. Verify: load the site locally, then confirm the beacon POST to \
         /collect/… returns HTTP 202 in the browser network tab.\n\
         6. Report what you changed."
    )
}

fn setup_response(
    config: Option<&FirstPartyConfig>,
    project: &Project,
    fallback_base: Option<&str>,
    receiving: bool,
) -> SetupResponse {
    match config {
        None => SetupResponse {
            enabled: false,
            site_key: None,
            ingest_base: None,
            ingest_url: None,
            snippet: None,
            agent_prompt: None,
            receiving,
        },
        Some(c) => {
            let base = c
                .ingest_base
                .clone()
                .or_else(|| fallback_base.map(str::to_owned))
                .unwrap_or_else(|| "http://localhost:3000".to_string());
            let base = base.trim_end_matches('/').to_string();
            let ingest_url = format!("{base}/collect/{}", c.site_key);
            let snippet = snippet_for(&ingest_url);
            let agent_prompt = agent_prompt_for(&project.name, &ingest_url, &snippet);
            SetupResponse {
                enabled: true,
                site_key: Some(c.site_key.clone()),
                ingest_base: c.ingest_base.clone(),
                ingest_url: Some(ingest_url),
                snippet: Some(snippet),
                agent_prompt: Some(agent_prompt),
                receiving,
            }
        }
    }
}

/// Base URL guess from the request's Host header — right for same-machine
/// dev, and the UI lets the user override it for anything public.
fn base_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| format!("http://{h}"))
}

async fn has_any_events(pool: &Pool<Sqlite>, project_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM analytics_events WHERE project_id = ?1 LIMIT 1",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await
    .map(|n| n > 0)
    .unwrap_or(false)
}

// ── Protected handlers ───────────────────────────────────────────────────────

async fn get_setup(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SetupResponse>, StatusCode> {
    let (pool, project) = project_and_pool(&state, &project_id).await?;
    let config = config_from_project(&project);
    let receiving = has_any_events(&pool, &project.id).await;
    Ok(Json(setup_response(
        config.as_ref(),
        &project,
        base_from_headers(&headers).as_deref(),
        receiving,
    )))
}

async fn enable_first_party(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<EnableRequest>>,
) -> Result<Json<SetupResponse>, StatusCode> {
    let (pool, project) = project_and_pool(&state, &project_id).await?;
    let requested_base = body
        .and_then(|Json(b)| b.ingest_base)
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());

    let mut config = config_from_project(&project).unwrap_or_else(|| FirstPartyConfig {
        site_key: mint_site_key(),
        ingest_base: None,
    });
    if requested_base.is_some() {
        config.ingest_base = requested_base;
    }

    // Read-modify-write the metadata bag, preserving unrelated keys.
    let mut metadata = if project.metadata_json.is_object() {
        project.metadata_json.clone()
    } else {
        serde_json::json!({})
    };
    metadata.as_object_mut().expect("object ensured").insert(
        METADATA_KEY.to_string(),
        serde_json::to_value(&config).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    let updated = projects::update_project(
        &pool,
        &project.id,
        UpdateProject {
            metadata_json: Some(metadata),
            ..Default::default()
        },
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .unwrap_or(project);

    let receiving = has_any_events(&pool, &updated.id).await;
    Ok(Json(setup_response(
        Some(&config),
        &updated,
        base_from_headers(&headers).as_deref(),
        receiving,
    )))
}

#[derive(Debug, Deserialize)]
struct StatsQuery {
    #[serde(default)]
    days: Option<u32>,
}

async fn first_party_stats(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Query(q): Query<StatsQuery>,
) -> Result<Json<FirstPartyStats>, StatusCode> {
    let (pool, project) = project_and_pool(&state, &project_id).await?;
    let enabled = config_from_project(&project).is_some();
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let since = format!("-{days} days");

    let (pageviews, visitors): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(DISTINCT visitor_hash)
         FROM analytics_events
         WHERE project_id = ?1 AND kind = 'pageview' AND created_at >= datetime('now', ?2)",
    )
    .bind(&project.id)
    .bind(&since)
    .fetch_one(&pool)
    .await
    .unwrap_or((0, 0));

    let events_last_5m: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM analytics_events
         WHERE project_id = ?1 AND created_at >= datetime('now', '-5 minutes')",
    )
    .bind(&project.id)
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

    let by_day: Vec<DayCount> = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT date(created_at), count(*), count(DISTINCT visitor_hash)
         FROM analytics_events
         WHERE project_id = ?1 AND kind = 'pageview' AND created_at >= datetime('now', ?2)
         GROUP BY date(created_at) ORDER BY date(created_at)",
    )
    .bind(&project.id)
    .bind(&since)
    .fetch_all(&pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(day, pageviews, visitors)| DayCount {
        day,
        pageviews,
        visitors,
    })
    .collect();

    let named = |rows: Vec<(String, i64)>| -> Vec<NamedCount> {
        rows.into_iter()
            .map(|(name, count)| NamedCount { name, count })
            .collect()
    };

    let top_pages = named(
        sqlx::query_as::<_, (String, i64)>(
            "SELECT path, count(*) FROM analytics_events
             WHERE project_id = ?1 AND kind = 'pageview' AND created_at >= datetime('now', ?2)
             GROUP BY path ORDER BY count(*) DESC LIMIT 10",
        )
        .bind(&project.id)
        .bind(&since)
        .fetch_all(&pool)
        .await
        .unwrap_or_default(),
    );

    let top_referrers = named(
        sqlx::query_as::<_, (String, i64)>(
            "SELECT referrer, count(*) FROM analytics_events
             WHERE project_id = ?1 AND kind = 'pageview' AND referrer IS NOT NULL
               AND referrer <> '' AND created_at >= datetime('now', ?2)
             GROUP BY referrer ORDER BY count(*) DESC LIMIT 10",
        )
        .bind(&project.id)
        .bind(&since)
        .fetch_all(&pool)
        .await
        .unwrap_or_default(),
    );

    let top_events = named(
        sqlx::query_as::<_, (String, i64)>(
            "SELECT coalesce(name, '(unnamed)'), count(*) FROM analytics_events
             WHERE project_id = ?1 AND kind = 'event' AND created_at >= datetime('now', ?2)
             GROUP BY name ORDER BY count(*) DESC LIMIT 10",
        )
        .bind(&project.id)
        .bind(&since)
        .fetch_all(&pool)
        .await
        .unwrap_or_default(),
    );

    let receiving = pageviews > 0 || has_any_events(&pool, &project.id).await;
    Ok(Json(FirstPartyStats {
        enabled,
        receiving,
        period_days: days,
        pageviews,
        visitors,
        events_last_5m,
        by_day,
        top_pages,
        top_referrers,
        top_events,
    }))
}

// ── Public collect endpoint ──────────────────────────────────────────────────

/// Rolling per-site-key rate limiter. Coarse by design: one bucket per key,
/// pruned lazily; protects the DB, not a billing meter.
fn rate_ok(site_key: &str) -> bool {
    static BUCKETS: OnceLock<Mutex<std::collections::HashMap<String, (std::time::Instant, u32)>>> =
        OnceLock::new();
    let buckets = BUCKETS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let Ok(mut map) = buckets.lock() else {
        return true;
    };
    let now = std::time::Instant::now();
    map.retain(|_, (start, _)| now.duration_since(*start).as_secs() < 120);
    let entry = map.entry(site_key.to_string()).or_insert((now, 0));
    if now.duration_since(entry.0).as_secs() >= 60 {
        *entry = (now, 0);
    }
    entry.1 += 1;
    entry.1 <= EVENTS_PER_MINUTE
}

#[derive(Debug, Deserialize)]
struct BeaconBody {
    /// "pv" (pageview) or "ev" (custom event).
    #[serde(default)]
    k: Option<String>,
    #[serde(default)]
    p: Option<String>,
    #[serde(default)]
    r: Option<String>,
    #[serde(default)]
    n: Option<String>,
}

fn clamp(s: Option<String>, max: usize) -> Option<String> {
    s.map(|v| v.chars().take(max).collect::<String>())
        .filter(|v| !v.is_empty())
}

/// Resolve a site key to its project id by scanning project metadata. Personal
/// scale (a handful of projects); the rate limiter fronts this.
async fn project_for_site_key(pool: &Pool<Sqlite>, site_key: &str) -> Option<String> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, metadata_json FROM projects WHERE metadata_json LIKE ?1")
            .bind(format!("%{site_key}%"))
            .fetch_all(pool)
            .await
            .ok()?;
    for (id, metadata) in rows {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&metadata) {
            if value
                .get(METADATA_KEY)
                .and_then(|c| c.get("siteKey"))
                .and_then(|k| k.as_str())
                == Some(site_key)
            {
                return Some(id);
            }
        }
    }
    None
}

async fn collect(
    State(state): State<Arc<AppState>>,
    Path(site_key): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    // Shape checks before any DB work.
    if site_key.len() != 32 || !site_key.chars().all(|c| c.is_ascii_hexdigit()) {
        return StatusCode::NOT_FOUND;
    }
    if body.len() > MAX_BODY_BYTES {
        return StatusCode::PAYLOAD_TOO_LARGE;
    }
    if !rate_ok(&site_key) {
        return StatusCode::TOO_MANY_REQUESTS;
    }
    // sendBeacon posts text/plain — parse from raw bytes, never via the JSON
    // extractor (which would 415 on the missing content type).
    let beacon: BeaconBody = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    let kind = match beacon.k.as_deref() {
        None | Some("pv") => "pageview",
        Some("ev") => "event",
        Some(_) => return StatusCode::BAD_REQUEST,
    };

    let Ok(pool) = state.session_manager().pool_clone().await else {
        return StatusCode::INTERNAL_SERVER_ERROR;
    };
    let Some(project_id) = project_for_site_key(&pool, &site_key).await else {
        return StatusCode::NOT_FOUND;
    };

    // Privacy-preserving daily visitor hash: UA + language + UTC day + key.
    // No IP, no cookie, rotates at midnight UTC.
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let lang = headers
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut hasher = Sha256::new();
    hasher.update(site_key.as_bytes());
    hasher.update(ua.as_bytes());
    hasher.update(lang.as_bytes());
    hasher.update(day.as_bytes());
    let visitor_hash = hex::encode(&hasher.finalize()[..16]);

    let path = clamp(beacon.p, 512).unwrap_or_else(|| "/".to_string());
    let referrer = clamp(beacon.r, 512);
    let name = clamp(beacon.n, 128);

    let inserted = sqlx::query(
        "INSERT INTO analytics_events (project_id, kind, path, referrer, name, visitor_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&project_id)
    .bind(kind)
    .bind(&path)
    .bind(&referrer)
    .bind(&name)
    .bind(&visitor_hash)
    .execute(&pool)
    .await;

    match inserted {
        Ok(_) => StatusCode::ACCEPTED,
        Err(e) => {
            tracing::warn!("analytics collect insert failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

// ── Routers ──────────────────────────────────────────────────────────────────

/// Bearer-protected management surface (merged into the protected group).
pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/projects/{project_id}/analytics/first_party",
            get(get_setup),
        )
        .route(
            "/api/projects/{project_id}/analytics/first_party/enable",
            post(enable_first_party),
        )
        .route(
            "/api/projects/{project_id}/analytics/first_party/stats",
            get(first_party_stats),
        )
        .with_state(state)
}

/// The beacon endpoint — mounted OUTSIDE bearer auth and the origin guard
/// (see module docs for the exposure bounds).
pub fn collect_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/collect/{site_key}", post(collect))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use serial_test::serial;
    use tower::ServiceExt;

    async fn request(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().uri(uri).method(method);
        let body = match body {
            Some(v) => {
                builder = builder.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let response = app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            serde_json::json!(null)
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| serde_json::json!(String::from_utf8_lossy(&bytes)))
        };
        (status, json)
    }

    /// Beacon POST with NO content-type header — exactly what sendBeacon sends.
    async fn beacon(app: &Router, uri: &str, body: serde_json::Value) -> StatusCode {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .method("POST")
                    .header("user-agent", "test-browser/1.0")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        response.status()
    }

    async fn test_app() -> (Router, Router, Pool<Sqlite>, Arc<AppState>) {
        let state = AppState::new(true).await.unwrap();
        let pool = state.session_manager().pool_clone().await.unwrap();
        (
            routes(state.clone()),
            collect_routes(state.clone()),
            pool,
            state,
        )
    }

    async fn seed_project(pool: &Pool<Sqlite>, name: &str) -> Project {
        projects::create_project(
            pool,
            projects::CreateProject {
                name: name.to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn enable_collect_stats_roundtrip() {
        let root = crate::test_support::test_root();
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);
        let (app, collect_app, pool, _state) = test_app().await;
        let project = seed_project(&pool, "fp-analytics-roundtrip").await;

        // Enable mints a key and returns the setup payload.
        let (status, setup) = request(
            &app,
            "POST",
            &format!("/api/projects/{}/analytics/first_party/enable", project.id),
            Some(serde_json::json!({ "ingestBase": "https://example.tail1234.ts.net" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let site_key = setup["siteKey"].as_str().unwrap().to_string();
        assert_eq!(site_key.len(), 32);
        assert!(setup["ingestUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://example.tail1234.ts.net/collect/"));
        assert!(setup["snippet"].as_str().unwrap().contains(&site_key));
        assert!(setup["agentPrompt"]
            .as_str()
            .unwrap()
            .contains("window.permagent.event"));
        assert_eq!(setup["receiving"], serde_json::json!(false));

        // Re-enable is idempotent: same key.
        let (_, setup2) = request(
            &app,
            "POST",
            &format!("/api/projects/{}/analytics/first_party/enable", project.id),
            None,
        )
        .await;
        assert_eq!(setup2["siteKey"].as_str().unwrap(), site_key);

        // Beacons ingest without auth and without a JSON content type.
        assert_eq!(
            beacon(
                &collect_app,
                &format!("/collect/{site_key}"),
                serde_json::json!({ "k": "pv", "p": "/pricing", "r": "https://news.ycombinator.com/" }),
            )
            .await,
            StatusCode::ACCEPTED
        );
        assert_eq!(
            beacon(
                &collect_app,
                &format!("/collect/{site_key}"),
                serde_json::json!({ "k": "ev", "p": "/pricing", "n": "signup" }),
            )
            .await,
            StatusCode::ACCEPTED
        );

        // Unknown key 404s; malformed body 400s.
        assert_eq!(
            beacon(
                &collect_app,
                &format!("/collect/{}", "0".repeat(32)),
                serde_json::json!({ "k": "pv" }),
            )
            .await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            beacon(
                &collect_app,
                &format!("/collect/{site_key}"),
                serde_json::json!("nope")
            )
            .await,
            StatusCode::BAD_REQUEST
        );

        // Stats aggregate what was ingested.
        let (status, stats) = request(
            &app,
            "GET",
            &format!(
                "/api/projects/{}/analytics/first_party/stats?days=7",
                project.id
            ),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["enabled"], serde_json::json!(true));
        assert_eq!(stats["receiving"], serde_json::json!(true));
        assert_eq!(stats["pageviews"], serde_json::json!(1));
        assert_eq!(stats["visitors"], serde_json::json!(1));
        assert_eq!(stats["topPages"][0]["name"], serde_json::json!("/pricing"));
        assert_eq!(
            stats["topReferrers"][0]["name"],
            serde_json::json!("https://news.ycombinator.com/")
        );
        assert_eq!(stats["topEvents"][0]["name"], serde_json::json!("signup"));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn disabled_project_reports_honestly() {
        let root = crate::test_support::test_root();
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);
        let (app, _collect_app, pool, _state) = test_app().await;
        let project = seed_project(&pool, "fp-analytics-disabled").await;

        let (status, setup) = request(
            &app,
            "GET",
            &format!("/api/projects/{}/analytics/first_party", project.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(setup["enabled"], serde_json::json!(false));
        assert_eq!(setup["siteKey"], serde_json::json!(null));

        let (status, stats) = request(
            &app,
            "GET",
            &format!("/api/projects/{}/analytics/first_party/stats", project.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["enabled"], serde_json::json!(false));
        assert_eq!(stats["receiving"], serde_json::json!(false));
        assert_eq!(stats["pageviews"], serde_json::json!(0));

        // Unknown project 404s.
        let (status, _) = request(
            &app,
            "GET",
            "/api/projects/does-not-exist/analytics/first_party",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
