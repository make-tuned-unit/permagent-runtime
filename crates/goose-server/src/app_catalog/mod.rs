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

        // The Inbox page lives inside Settings (2026-08 Console
        // consolidation): the stable name still resolves, and the entry's
        // fixed `section` deep-links to the right Settings pane.
        let inbox = catalog
            .find_by_name("Inbox")
            .expect("Inbox must be in the navigate_app catalog");
        assert_eq!(inbox.tool_type, "settings");
        assert_eq!(inbox.panel_type, "overlay");
        assert_eq!(inbox.section.as_deref(), Some("inbox"));

        // Skills is an overlay-only surface (no seeded workspace hosts it), so
        // navigate_app("Skills") depends entirely on this catalog entry.
        let skills = catalog
            .find_by_name("Skills")
            .expect("Skills must be in the navigate_app catalog");
        assert_eq!(skills.tool_type, "skills");
        assert_eq!(skills.panel_type, "overlay");
    }

    /// Sessions + Trace live inside Settings now (2026-08 Console
    /// consolidation): the stable names still resolve via the catalog, but
    /// each entry routes to the Settings overlay with a fixed `section`
    /// (`sessions` / `activity`) so useAppNavigate lands
    /// `setActivePanel('settings')` + `pendingSettingsSection`.
    #[test]
    fn sessions_and_trace_are_overlay_navigable() {
        let catalog: AppCatalog =
            serde_yaml::from_str(CATALOG_YAML).expect("catalog.yaml must parse");

        let sessions = catalog
            .find_by_name("Sessions")
            .expect("Sessions must be in the navigate_app catalog");
        assert_eq!(sessions.tool_type, "settings");
        assert_eq!(sessions.panel_type, "overlay");
        assert_eq!(sessions.section.as_deref(), Some("sessions"));

        let trace = catalog
            .find_by_name("Trace")
            .expect("Trace must be in the navigate_app catalog");
        assert_eq!(trace.tool_type, "settings");
        assert_eq!(trace.panel_type, "overlay");
        assert_eq!(trace.section.as_deref(), Some("activity"));
    }

    /// Every teaching step that opens a surface must name a tab the catalog can
    /// actually navigate to — a lesson pointing at a nonexistent tab walks the
    /// user into a dead click (the wave-1 "Home" regression: the dashboard
    /// lesson navigated to "Home", which is not a catalog entry).
    #[test]
    fn every_teaching_step_opens_a_real_catalog_tab() {
        let catalog: AppCatalog =
            serde_yaml::from_str(CATALOG_YAML).expect("catalog.yaml must parse");

        let mut checked = 0usize;
        let mut assert_steps =
            |owner: &str, steps: &[permagent::agents::self_knowledge::TeachingStep]| {
                for step in steps {
                    if let Some(surface) = step.open_surface {
                        assert!(
                            catalog.find_by_name(surface.tab).is_some(),
                            "teaching step '{}' of '{}' opens tab '{}' which is not in \
                             catalog.yaml — the lesson would dead-end",
                            step.title,
                            owner,
                            surface.tab
                        );
                        checked += 1;
                    }
                }
            };

        for (name, def) in permagent::agents::platform_extensions::PLATFORM_EXTENSIONS.iter() {
            assert_steps(name, def.teaching);
        }
        for d in permagent::agents::self_knowledge::WORKER_DESCRIPTORS {
            assert_steps(d.name, d.teaching);
        }
        for d in permagent::agents::self_knowledge::GUARD_DESCRIPTORS {
            assert_steps(d.name, d.teaching);
        }
        for d in permagent::agents::self_knowledge::SURFACE_DESCRIPTORS {
            assert_steps(d.name, d.teaching);
        }

        assert!(
            checked > 0,
            "no teaching step carried an open_surface — the guard is vacuous"
        );
    }

    /// The Governance surface was removed (2026-08 ruling): its panels merged
    /// into Settings (Spend / Sovereignty / Models / Autonomy). The catalog
    /// must no longer offer it, or the agent would navigate users to a surface
    /// the app doesn't mount.
    #[test]
    fn governance_is_gone_from_the_catalog() {
        let catalog: AppCatalog =
            serde_yaml::from_str(CATALOG_YAML).expect("catalog.yaml must parse");
        assert!(
            catalog.find_by_name("Governance").is_none(),
            "Governance was folded into Settings and must not be navigable"
        );
    }
}
