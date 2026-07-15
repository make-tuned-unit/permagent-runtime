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

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded catalog must parse and expose the tabs the UI seeds so
    /// `navigate_app("<Tab>")` resolves. Guards the wiring-audit regression (#6)
    /// where "Grow" was a seeded sidebar tab but absent from the catalog, so
    /// navigate_app("Grow") was rejected. Also covers the Downloads inbox (#4),
    /// reachable as an overlay.
    #[test]
    fn catalog_parses_and_grow_and_inbox_are_navigable() {
        let catalog: AppCatalog =
            serde_yaml::from_str(CATALOG_YAML).expect("catalog.yaml must parse");

        let grow = catalog
            .find_by_name("Grow")
            .expect("Grow must be in the navigate_app catalog");
        assert_eq!(grow.tool_type, "grow");

        let inbox = catalog
            .find_by_name("Inbox")
            .expect("Inbox must be in the navigate_app catalog");
        assert_eq!(inbox.tool_type, "inbox");
        assert_eq!(inbox.panel_type, "overlay");
    }
}
