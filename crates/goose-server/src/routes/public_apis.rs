//! Public data sources catalog (Settings → Data sources).
//!
//! Browse by category, toggle a source on, store a key when needed. Enabling
//! a source makes it callable immediately via `public_api_call`.

use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use permagent::public_apis::{self, CatalogEntry, CategoryView};
use serde::{Deserialize, Serialize};

struct ApiError(StatusCode, String);

impl From<(StatusCode, String)> for ApiError {
    fn from((status, message): (StatusCode, String)) -> Self {
        ApiError(status, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "message": self.1 }))).into_response()
    }
}

pub fn routes() -> Router {
    Router::new()
        .route("/api/public-apis/categories", get(list_categories))
        .route("/api/public-apis/catalog", get(list_catalog))
        .route("/api/public-apis/enabled", get(list_enabled))
        .route("/api/public-apis/{slug}/enable", post(enable_source))
        .route(
            "/api/public-apis/{slug}/key",
            post(set_key).delete(delete_key),
        )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogResponse {
    categories: Vec<CategoryView>,
    entries: Vec<CatalogRow>,
    enabled: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogRow {
    #[serde(flatten)]
    entry: CatalogEntry,
    enabled: bool,
    key_present: bool,
}

fn rows_for(category: Option<&str>) -> Vec<CatalogRow> {
    let enabled = public_apis::enabled_slugs();
    public_apis::catalog()
        .iter()
        .filter(|e| {
            category
                .map(|c| e.category.eq_ignore_ascii_case(c))
                .unwrap_or(true)
        })
        .map(|e| CatalogRow {
            enabled: enabled.iter().any(|s| s == &e.slug),
            key_present: public_apis::has_key(&e.slug),
            entry: e.clone(),
        })
        .collect()
}

async fn list_categories() -> Json<Vec<CategoryView>> {
    Json(public_apis::categories())
}

#[derive(Debug, Deserialize)]
struct CatalogQuery {
    category: Option<String>,
}

async fn list_catalog(
    axum::extract::Query(q): axum::extract::Query<CatalogQuery>,
) -> Json<CatalogResponse> {
    let category = q
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    Json(CatalogResponse {
        categories: public_apis::categories(),
        entries: if category.is_some() {
            rows_for(category)
        } else {
            Vec::new()
        },
        enabled: public_apis::enabled_slugs(),
    })
}

async fn list_enabled() -> Json<Vec<CatalogRow>> {
    let enabled = public_apis::enabled_slugs();
    Json(
        public_apis::catalog()
            .iter()
            .filter(|e| enabled.iter().any(|s| s == &e.slug))
            .map(|e| CatalogRow {
                enabled: true,
                key_present: public_apis::has_key(&e.slug),
                entry: e.clone(),
            })
            .collect(),
    )
}

#[derive(Debug, Deserialize)]
struct EnableBody {
    enabled: bool,
}

async fn enable_source(
    Path(slug): Path<String>,
    Json(body): Json<EnableBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let slugs =
        public_apis::set_enabled(&slug, body.enabled).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let suggested = public_apis::find(&slug)
        .map(|e| e.suggested_agents.clone())
        .unwrap_or_default();
    Ok(Json(serde_json::json!({
        "slug": slug,
        "enabled": body.enabled,
        "enabledSlugs": slugs,
        "suggestedAgents": suggested,
        "flowsImmediately": true,
    })))
}

#[derive(Debug, Deserialize)]
struct KeyBody {
    value: String,
}

async fn set_key(
    Path(slug): Path<String>,
    Json(body): Json<KeyBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    public_apis::find(&slug).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("unknown data source `{slug}`"),
        )
    })?;
    let value = body.value.trim();
    if value.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "key is empty".into()).into());
    }
    permagent::config::Config::global()
        .set_secret(&public_apis::secret_key(&slug), &value.to_string())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "slug": slug, "keyPresent": true }),
    ))
}

async fn delete_key(Path(slug): Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    public_apis::find(&slug).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("unknown data source `{slug}`"),
        )
    })?;
    let _ = permagent::config::Config::global().delete_secret(&public_apis::secret_key(&slug));
    Ok(Json(
        serde_json::json!({ "slug": slug, "keyPresent": false }),
    ))
}
