//! Grow analytics lens — connection management + read-only stats fetch.
//!
//! Ruled decision (2026-07-20): API-client-to-existing-analytics-account, not
//! a self-hosted collector. The provider client lives in
//! [`crate::analytics`]; this file is the HTTP surface the Grow tab's
//! "connect analytics" form and funnel/tiles talk to:
//!
//!   GET    /api/projects/{id}/analytics/connection       — status (never echoes the key)
//!   PUT    /api/projects/{id}/analytics/connection       — save {provider, baseUrl, siteId, apiKey}
//!   DELETE /api/projects/{id}/analytics/connection       — disconnect (config + secret)
//!   POST   /api/projects/{id}/analytics/connection/test  — live round-trip check
//!   GET    /api/projects/{id}/analytics/stats?period=30d — visitors/pageviews for the funnel
//!
//! Storage split: the non-secret connection config rides the project's
//! `metadata_json` bag (like other per-project metadata); the API key goes to
//! the global secret store (keyring/file) under a per-project key. Fetches
//! fail closed under sovereign mode — see `crate::analytics` for why.
//!
//! Auth: bearer-token middleware (merged into the protected group).

use crate::analytics::{
    api_key_secret_key, connection_from_metadata, fetch_stats, normalize_base_url,
    validate_connection, AnalyticsConnection, AnalyticsError, AnalyticsProvider, StatsPeriod,
    METADATA_KEY,
};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use permagent::config::Config;
use permagent::projects::{self, Project, UpdateProject};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::sync::{Arc, OnceLock};

/// One shared client: connection pooling + a hard timeout so a dead analytics
/// host can't wedge a request handler.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("reqwest client builds")
    })
}

// ── Wire types ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionStatus {
    connected: bool,
    provider: Option<AnalyticsProvider>,
    base_url: Option<String>,
    site_id: Option<String>,
    /// Whether an API key is stored for this project. The key itself is never
    /// returned.
    has_api_key: bool,
}

impl ConnectionStatus {
    fn from(conn: Option<&AnalyticsConnection>, has_api_key: bool) -> Self {
        ConnectionStatus {
            connected: conn.is_some(),
            provider: conn.map(|c| c.provider),
            base_url: conn.map(|c| c.base_url.clone()),
            site_id: conn.map(|c| c.site_id.clone()),
            has_api_key,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveConnectionRequest {
    provider: AnalyticsProvider,
    base_url: String,
    #[serde(default)]
    site_id: Option<String>,
    /// Optional on edit: omitting it keeps the already-stored key, so the UI
    /// never needs to echo a secret back into the form.
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestConnectionResponse {
    ok: bool,
    visitors: Option<u64>,
    pageviews: Option<u64>,
    error: Option<String>,
}

/// Stats for the funnel/tiles. Fetch failures (provider down, bad key,
/// sovereign refusal) are expected states the UI must render honestly, so
/// they ride a 200 with `error` set rather than an opaque 5xx.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsResponse {
    connected: bool,
    provider: Option<AnalyticsProvider>,
    period_days: Option<u32>,
    visitors: Option<u64>,
    pageviews: Option<u64>,
    error: Option<String>,
}

impl StatsResponse {
    fn not_connected() -> Self {
        StatsResponse {
            connected: false,
            provider: None,
            period_days: None,
            visitors: None,
            pageviews: None,
            error: None,
        }
    }
}

// ── Shared helpers ───────────────────────────────────────────────────────────

/// Error responses carry a JSON body (`{"error": …}`): the UI's `apiFetch`
/// surfaces `error`/`message` from JSON bodies, while a plain-text body would
/// render as "Unknown error" in the connect form.
type ApiError = (StatusCode, Json<serde_json::Value>);

fn api_err(status: StatusCode, msg: impl Into<String>) -> ApiError {
    (status, Json(serde_json::json!({ "error": msg.into() })))
}

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

fn stored_api_key(project_id: &str) -> Option<String> {
    Config::global()
        .get_secret::<String>(&api_key_secret_key(project_id))
        .ok()
        .filter(|k| !k.is_empty())
}

/// Write the (possibly removed) connection back into the project's metadata
/// bag. Read-modify-write on the whole bag, preserving unrelated keys.
async fn write_connection_metadata(
    pool: &Pool<Sqlite>,
    project: &Project,
    conn: Option<&AnalyticsConnection>,
) -> Result<(), String> {
    let mut metadata = if project.metadata_json.is_object() {
        project.metadata_json.clone()
    } else {
        serde_json::json!({})
    };
    let obj = metadata.as_object_mut().expect("object ensured above");
    match conn {
        Some(c) => {
            obj.insert(
                METADATA_KEY.to_string(),
                serde_json::to_value(c).map_err(|e| e.to_string())?,
            );
        }
        None => {
            obj.remove(METADATA_KEY);
        }
    }
    projects::update_project(
        pool,
        &project.id,
        UpdateProject {
            metadata_json: Some(metadata),
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn get_connection(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Json<ConnectionStatus>, StatusCode> {
    let (_pool, project) = project_and_pool(&state, &project_id).await?;
    let conn = connection_from_metadata(&project.metadata_json);
    let has_key = stored_api_key(&project.id).is_some();
    Ok(Json(ConnectionStatus::from(conn.as_ref(), has_key)))
}

async fn save_connection(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(req): Json<SaveConnectionRequest>,
) -> Result<Json<ConnectionStatus>, ApiError> {
    let (pool, project) = project_and_pool(&state, &project_id)
        .await
        .map_err(|s| api_err(s, "project lookup failed"))?;

    let base_url = normalize_base_url(&req.base_url)
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    let conn = AnalyticsConnection {
        provider: req.provider,
        base_url,
        site_id: req.site_id.unwrap_or_default().trim().to_string(),
    };
    validate_connection(&conn).map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;

    // Key handling: a new key replaces the stored one; omitted/blank keeps it
    // (edit flow). A first-time save with no key at all is refused — a
    // connection that can never fetch would be dead UI.
    let new_key = req
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty());
    match new_key {
        Some(key) => {
            Config::global()
                .set_secret(&api_key_secret_key(&project.id), &key.to_string())
                .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        None => {
            if stored_api_key(&project.id).is_none() {
                return Err(api_err(
                    StatusCode::BAD_REQUEST,
                    "an API key is required to connect analytics",
                ));
            }
        }
    }

    write_connection_metadata(&pool, &project, Some(&conn))
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(ConnectionStatus::from(Some(&conn), true)))
}

async fn delete_connection(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Json<ConnectionStatus>, ApiError> {
    let (pool, project) = project_and_pool(&state, &project_id)
        .await
        .map_err(|s| api_err(s, "project lookup failed"))?;

    write_connection_metadata(&pool, &project, None)
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    // Removing an absent key is a no-op in the secret store; a real storage
    // failure must surface — a disconnect that silently keeps the secret
    // would betray "disconnect".
    Config::global()
        .delete_secret(&api_key_secret_key(&project.id))
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(ConnectionStatus::from(None, false)))
}

async fn test_connection(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Json<TestConnectionResponse>, ApiError> {
    let (_pool, project) = project_and_pool(&state, &project_id)
        .await
        .map_err(|s| api_err(s, "project lookup failed"))?;

    let Some(conn) = connection_from_metadata(&project.metadata_json) else {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "analytics is not connected for this project",
        ));
    };
    let Some(key) = stored_api_key(&project.id) else {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "no API key stored — save the connection with a key first",
        ));
    };

    // A short window keeps the test cheap; success proves URL + key + site.
    let result = fetch_stats(http_client(), &conn, &key, StatsPeriod::Days7).await;
    Ok(Json(match result {
        Ok(stats) => TestConnectionResponse {
            ok: true,
            visitors: stats.visitors,
            pageviews: stats.pageviews,
            error: None,
        },
        Err(e) => TestConnectionResponse {
            ok: false,
            visitors: None,
            pageviews: None,
            error: Some(e.to_string()),
        },
    }))
}

#[derive(Debug, Deserialize)]
struct StatsQuery {
    #[serde(default)]
    period: Option<String>,
}

async fn get_stats(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Query(q): Query<StatsQuery>,
) -> Result<Json<StatsResponse>, ApiError> {
    let (_pool, project) = project_and_pool(&state, &project_id)
        .await
        .map_err(|s| api_err(s, "project lookup failed"))?;

    let period = match q.period.as_deref() {
        None => StatsPeriod::Days30,
        Some(p) => StatsPeriod::parse(p)
            .ok_or_else(|| api_err(StatusCode::BAD_REQUEST, "period must be one of: 7d, 30d"))?,
    };

    let Some(conn) = connection_from_metadata(&project.metadata_json) else {
        return Ok(Json(StatsResponse::not_connected()));
    };
    let provider = Some(conn.provider);

    let fetched = match stored_api_key(&project.id) {
        None => Err(AnalyticsError::InvalidConfig(
            "no API key stored — reconnect analytics".to_string(),
        )),
        Some(key) => fetch_stats(http_client(), &conn, &key, period).await,
    };

    Ok(Json(match fetched {
        Ok(stats) => StatsResponse {
            connected: true,
            provider,
            period_days: Some(stats.period_days),
            visitors: stats.visitors,
            pageviews: stats.pageviews,
            error: None,
        },
        Err(e) => StatsResponse {
            connected: true,
            provider,
            period_days: Some(period.days()),
            visitors: None,
            pageviews: None,
            error: Some(e.to_string()),
        },
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/projects/{project_id}/analytics/connection",
            get(get_connection)
                .put(save_connection)
                .delete(delete_connection),
        )
        .route(
            "/api/projects/{project_id}/analytics/connection/test",
            post(test_connection),
        )
        .route("/api/projects/{project_id}/analytics/stats", get(get_stats))
        .with_state(state)
}

// ── Tests ────────────────────────────────────────────────────────────────────
//
// Route tests build a real AppState (DB under a temp PERMAGENT_PATH_ROOT), so
// they are #[serial] per the standing rule. Secrets ride the process-global
// Config store under a per-project (UUID) key, so each run touches only its
// own throwaway entries and cleans them up via the DELETE route.

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

    async fn test_app() -> (Router, Pool<Sqlite>) {
        let state = AppState::new(true).await.unwrap();
        let pool = state.session_manager().pool_clone().await.unwrap();
        (routes(state), pool)
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
    async fn connection_lifecycle_roundtrip() {
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(home.path().to_str().unwrap())),
            ("SOVEREIGN_MODE", Some("false")),
        ]);
        permagent::sovereignty::invalidate_global_mode_cache();
        let (app, pool) = test_app().await;
        let project = seed_project(&pool, "Roundtrip").await;
        let base = format!("/api/projects/{}/analytics/connection", project.id);

        // Fresh project → honestly disconnected.
        let (status, body) = request(&app, "GET", &base, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["connected"], serde_json::json!(false));
        assert_eq!(body["hasApiKey"], serde_json::json!(false));

        // First save without a key is refused — that connection could never fetch.
        let (status, _) = request(
            &app,
            "PUT",
            &base,
            Some(serde_json::json!({
                "provider": "plausible",
                "baseUrl": "https://plausible.example.com",
                "siteId": "example.com",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Save with a key: normalized base URL, hasApiKey, and the key itself
        // must never be echoed anywhere in the response.
        let (status, body) = request(
            &app,
            "PUT",
            &base,
            Some(serde_json::json!({
                "provider": "plausible",
                "baseUrl": "https://plausible.example.com/",
                "siteId": "example.com",
                "apiKey": "sekrit-abc123",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["connected"], serde_json::json!(true));
        assert_eq!(body["provider"], serde_json::json!("plausible"));
        assert_eq!(
            body["baseUrl"],
            serde_json::json!("https://plausible.example.com"),
            "trailing slash must be normalized away"
        );
        assert_eq!(body["hasApiKey"], serde_json::json!(true));
        assert!(
            !body.to_string().contains("sekrit-abc123"),
            "API key must never be echoed"
        );

        // Read-back agrees, still without the key.
        let (status, body) = request(&app, "GET", &base, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["connected"], serde_json::json!(true));
        assert_eq!(body["siteId"], serde_json::json!("example.com"));
        assert_eq!(body["hasApiKey"], serde_json::json!(true));
        assert!(!body.to_string().contains("sekrit-abc123"));

        // Edit without re-entering the key keeps the stored one.
        let (status, body) = request(
            &app,
            "PUT",
            &base,
            Some(serde_json::json!({
                "provider": "goatcounter",
                "baseUrl": "https://me.goatcounter.com",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["provider"], serde_json::json!("goatcounter"));
        assert_eq!(body["hasApiKey"], serde_json::json!(true));

        // The config landed in the project's metadata bag (persistence seam).
        let stored = projects::get_project(&pool, &project.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.metadata_json[METADATA_KEY]["provider"],
            serde_json::json!("goatcounter")
        );

        // Disconnect removes config AND the stored key.
        let (status, body) = request(&app, "DELETE", &base, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["connected"], serde_json::json!(false));
        assert_eq!(body["hasApiKey"], serde_json::json!(false));
        let (_, body) = request(&app, "GET", &base, None).await;
        assert_eq!(body["connected"], serde_json::json!(false));
        assert_eq!(body["hasApiKey"], serde_json::json!(false));

        // Unrelated metadata keys survive the connect/disconnect cycle.
        let stored = projects::get_project(&pool, &project.id)
            .await
            .unwrap()
            .unwrap();
        assert!(stored.metadata_json.get(METADATA_KEY).is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn save_validates_input_and_unknown_project_is_404() {
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(home.path().to_str().unwrap())),
        ]);
        let (app, pool) = test_app().await;
        let project = seed_project(&pool, "Validate").await;
        let base = format!("/api/projects/{}/analytics/connection", project.id);

        // Scheme-less base URL → 400 at save time, not a confusing fetch error.
        let (status, _) = request(
            &app,
            "PUT",
            &base,
            Some(serde_json::json!({
                "provider": "plausible",
                "baseUrl": "plausible.example.com",
                "siteId": "example.com",
                "apiKey": "k",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Plausible without a site_id → 400.
        let (status, _) = request(
            &app,
            "PUT",
            &base,
            Some(serde_json::json!({
                "provider": "plausible_v2",
                "baseUrl": "https://plausible.io",
                "apiKey": "k",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Unknown project → 404 across the surface.
        let (status, _) = request(
            &app,
            "GET",
            "/api/projects/nope-not-real/analytics/connection",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = request(
            &app,
            "GET",
            "/api/projects/nope-not-real/analytics/stats",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn stats_honest_when_disconnected_and_refuses_under_sovereign_mode() {
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(home.path().to_str().unwrap())),
            ("SOVEREIGN_MODE", Some("true")),
        ]);
        permagent::sovereignty::invalidate_global_mode_cache();
        let (app, pool) = test_app().await;
        let project = seed_project(&pool, "Sovereign").await;
        let conn_uri = format!("/api/projects/{}/analytics/connection", project.id);
        let stats_uri = format!("/api/projects/{}/analytics/stats", project.id);

        // Disconnected → connected:false, no fabricated numbers.
        let (status, body) = request(&app, "GET", &stats_uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["connected"], serde_json::json!(false));
        assert_eq!(body["visitors"], serde_json::json!(null));

        // Invalid period → 400.
        let (status, _) = request(&app, "GET", &format!("{stats_uri}?period=90d"), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Connect (a local write — allowed under sovereign mode)…
        let (status, body) = request(
            &app,
            "PUT",
            &conn_uri,
            Some(serde_json::json!({
                "provider": "plausible",
                // Port 1: if the sovereign guard ever failed we'd see a
                // network error, not the refusal message.
                "baseUrl": "http://127.0.0.1:1",
                "siteId": "example.com",
                "apiKey": "k",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // …but the fetch itself must refuse: cloud egress under sovereign mode.
        let (status, body) = request(&app, "GET", &stats_uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["connected"], serde_json::json!(true));
        assert_eq!(body["visitors"], serde_json::json!(null));
        let err = body["error"].as_str().unwrap_or_default();
        assert!(
            err.contains("sovereign"),
            "expected sovereign refusal: {err}"
        );

        // The test endpoint refuses the same way (ok:false, honest reason).
        let test_uri = format!("{conn_uri}/test");
        let (status, body) = request(&app, "POST", &test_uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], serde_json::json!(false));
        let err = body["error"].as_str().unwrap_or_default();
        assert!(
            err.contains("sovereign"),
            "expected sovereign refusal: {err}"
        );

        // Cleanup: drop the throwaway secret + config.
        let (status, _) = request(&app, "DELETE", &conn_uri, None).await;
        assert_eq!(status, StatusCode::OK);
        permagent::sovereignty::invalidate_global_mode_cache();
    }
}
