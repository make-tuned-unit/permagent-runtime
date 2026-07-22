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
    /// navigate_app("Grow") was rejected. Also covers the Downloads inbox (#4)
    /// and the Skills library — both reachable as overlays with no sidebar tab,
    /// so the catalog entry is the ONLY thing that makes navigate_app resolve.
    #[test]
    fn catalog_parses_and_grow_inbox_skills_are_navigable() {
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

        // Skills is an overlay-only surface (no seeded workspace hosts it), so
        // navigate_app("Skills") depends entirely on this catalog entry.
        let skills = catalog
            .find_by_name("Skills")
            .expect("Skills must be in the navigate_app catalog");
        assert_eq!(skills.tool_type, "skills");
        assert_eq!(skills.panel_type, "overlay");
    }

    /// Sessions + Trace are overlay-only surfaces (no seeded workspace hosts
    /// either), so — exactly like Inbox and Skills — the catalog entry is the
    /// ONLY thing that makes navigate_app resolve them, and `panel_type` must be
    /// `overlay` so useAppNavigate routes them to `setActivePanel(...)` rather
    /// than the no-op workspace-host search.
    #[test]
    fn sessions_and_trace_are_overlay_navigable() {
        let catalog: AppCatalog =
            serde_yaml::from_str(CATALOG_YAML).expect("catalog.yaml must parse");

        let sessions = catalog
            .find_by_name("Sessions")
            .expect("Sessions must be in the navigate_app catalog");
        assert_eq!(sessions.tool_type, "sessions");
        assert_eq!(sessions.panel_type, "overlay");

        let trace = catalog
            .find_by_name("Trace")
            .expect("Trace must be in the navigate_app catalog");
        assert_eq!(trace.tool_type, "trace");
        assert_eq!(trace.panel_type, "overlay");
    }

    /// Governance is an overlay-only surface (no seeded workspace hosts it), so
    /// — like Inbox/Skills/Sessions/Trace — the catalog entry is the ONLY thing
    /// that makes `navigate_app("Governance")` resolve, and it must route as an
    /// overlay so useAppNavigate lands it via `setActivePanel('governance')`.
    #[test]
    fn governance_is_overlay_navigable() {
        let catalog: AppCatalog =
            serde_yaml::from_str(CATALOG_YAML).expect("catalog.yaml must parse");

        let gov = catalog
            .find_by_name("Governance")
            .expect("Governance must be in the navigate_app catalog");
        assert_eq!(gov.tool_type, "governance");
        assert_eq!(gov.panel_type, "overlay");
    }
}
