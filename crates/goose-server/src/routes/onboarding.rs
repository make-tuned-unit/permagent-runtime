//! Agent-led onboarding status endpoint.
//!
//! Read-only view over the feature-usage tracker
//! ([`permagent::agents::self_knowledge::usage`]): what the user has actually
//! engaged, and the ranked "learn next" list (`inventory − used`). This is the
//! same authoritative data the agent reasons over — the Learn-next UI surface
//! renders it, and every `learn_next` entry maps to a real descriptor + a real
//! navigable tab (enforced by tests in the `permagent` crate).

use axum::{routing::get, Json, Router};
use permagent::agents::self_knowledge::{find_descriptor, teachable, usage};
use serde::Serialize;

/// One capability the user has engaged.
#[derive(Serialize)]
struct UsedItem {
    id: String,
    display_name: String,
    /// Engagements observed since the last persist (durable fields are the
    /// timestamps below).
    count: u64,
    /// The agent has explicitly walked the user through this feature.
    taught: bool,
    first_used: String,
    last_used: String,
}

/// One capability the user has NOT engaged yet, with the real surface to open.
#[derive(Serialize)]
struct LearnNextItem {
    id: String,
    display_name: String,
    what_it_does: String,
    why_it_matters: String,
    /// App-catalog tab the agent opens to teach it.
    tab: String,
    section: Option<String>,
}

#[derive(Serialize)]
struct Totals {
    used: usize,
    teachable: usize,
}

#[derive(Serialize)]
struct StatusResponse {
    used: Vec<UsedItem>,
    learn_next: Vec<LearnNextItem>,
    totals: Totals,
}

/// `GET /api/onboarding/status` — usage + ranked learn-next.
async fn get_status() -> Json<StatusResponse> {
    let used: Vec<UsedItem> = usage::usage_snapshot()
        .into_iter()
        .map(|(id, r)| {
            let display_name = find_descriptor(&id)
                .map(|d| d.display_name.to_string())
                .unwrap_or_else(|| id.clone());
            UsedItem {
                id,
                display_name,
                count: r.count,
                taught: r.taught,
                first_used: r.first_used.to_rfc3339(),
                last_used: r.last_used.to_rfc3339(),
            }
        })
        .collect();

    let learn_next: Vec<LearnNextItem> = teachable::learn_next()
        .into_iter()
        .filter_map(|t| {
            let d = find_descriptor(t.id)?;
            Some(LearnNextItem {
                id: t.id.to_string(),
                display_name: d.display_name.to_string(),
                what_it_does: d.what_it_does.to_string(),
                why_it_matters: d.why_it_matters.to_string(),
                tab: t.surface.tab.to_string(),
                section: t.surface.section.map(|s| s.to_string()),
            })
        })
        .collect();

    let totals = Totals {
        used: used.len(),
        teachable: teachable::teachable_ids().len(),
    };

    Json(StatusResponse {
        used,
        learn_next,
        totals,
    })
}

pub fn routes() -> Router {
    Router::new().route("/api/onboarding/status", get(get_status))
}

#[cfg(test)]
mod tests {
    use permagent::agents::self_knowledge::{find_descriptor, teachable};

    /// The "no fake lesson" guarantee at the mounted-surface layer: every
    /// teachable capability's navigate target must be a tab in the **shipped**
    /// app catalog (parsed from catalog.yaml), not just the crate-local
    /// NAV_CATALOG_TABS list. This is the goose-server half of the invariant —
    /// it fails the build if the teachable set ever references a tab the app
    /// does not actually mount.
    #[test]
    fn every_teachable_surface_is_in_the_shipped_catalog() {
        let catalog = crate::app_catalog::init();
        let tab_names: std::collections::HashSet<String> =
            catalog.tabs.iter().map(|t| t.name.to_lowercase()).collect();

        for t in teachable::TEACHABLE {
            assert!(
                tab_names.contains(&t.surface.tab.to_lowercase()),
                "teachable id {:?} opens tab {:?}, which is not in the shipped app catalog \
                 (tabs: {:?})",
                t.id,
                t.surface.tab,
                catalog.tabs.iter().map(|x| &x.name).collect::<Vec<_>>()
            );
            // And the lesson content is real too.
            assert!(
                find_descriptor(t.id).is_some(),
                "teachable id {:?} has no self-knowledge descriptor",
                t.id
            );
        }
    }
}
