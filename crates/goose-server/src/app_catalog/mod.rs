//! App Catalog loader and HTTP endpoint.
//!
//! Parses the embedded catalog.yaml at startup and serves it via
//! `GET /api/app/catalog`.

use axum::{extract::State, routing::get, Json, Router};
use permagent::app_catalog::{set_global_catalog, AppCatalog};
use std::sync::Arc;

use crate::state::AppState;

/// Embedded catalog YAML — compiled into the binary.
const CATALOG_YAML: &str = include_str!("catalog.yaml");

/// Parse the catalog YAML and set the global. Returns the Arc for AppState.
pub fn init() -> Arc<AppCatalog> {
    let catalog: AppCatalog =
        serde_yaml::from_str(CATALOG_YAML).expect("catalog.yaml must be valid");
    let arc = Arc::new(catalog);
    set_global_catalog(arc.clone());
    arc
}

/// Register the catalog route.
pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/app/catalog", get(get_catalog))
        .with_state(state)
}

async fn get_catalog(State(state): State<Arc<AppState>>) -> Json<AppCatalog> {
    let catalog = state.app_catalog.as_ref().clone();
    Json(catalog)
}
