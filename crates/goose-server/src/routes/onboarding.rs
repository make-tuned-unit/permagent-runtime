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

    /// Everything in the app must be queryable by the agent.
    ///
    /// This is the enforcement half of that ruling. Every tab the app actually
    /// ships must either have an `observe_app` aspect or appear in
    /// `NOT_YET_OBSERVABLE` with a reason — a new tab cannot ship without a
    /// deliberate choice between the two, so coverage cannot rot quietly.
    ///
    /// It exists because of a real failure: the user asked their agent about
    /// growth actions it had recommended and got nothing. `growth_actions` held
    /// four rows; no tool could read them. The agent could DESCRIBE the Grow tab
    /// from a static self-knowledge descriptor while being unable to see
    /// anything in it, and no test anywhere objected. Editorial prose about a
    /// surface is not the same as being able to read it, and only the prose was
    /// ever checked.
    ///
    /// This is the mirror of `every_teachable_surface_is_in_the_shipped_catalog`
    /// below: that one proves the agent never teaches a tab that does not exist;
    /// this one proves the app never ships a tab the agent cannot read.
    #[test]
    fn every_shipped_tab_is_observable_or_exempt() {
        use permagent::agents::platform_extensions::app_perception::OBSERVABLE_SURFACES;

        /// Tabs with no `observe_app` aspect yet, and why. SHRINK THIS LIST.
        /// An entry here is a promise not kept, not a design decision — the
        /// agent cannot answer questions about these surfaces from its own
        /// data, and will either say so or, worse, guess.
        const NOT_YET_OBSERVABLE: &[(&str, &str)] = &[
            (
                "build",
                "coding sessions + browser state have no aspect yet",
            ),
            (
                "automate",
                "scheduled jobs and their run history have no aspect yet",
            ),
            ("brain", "memory/recall state has no aspect yet"),
            ("world", "world-view worker state has no aspect yet"),
            ("inbox", "decision inbox has no aspect yet"),
            ("skills", "installed/proposed skills have no aspect yet"),
            ("trace", "execution trace has no aspect yet"),
            ("settings", "configuration state has no aspect yet"),
        ];

        let catalog = crate::app_catalog::init();
        let observable: std::collections::HashSet<String> = OBSERVABLE_SURFACES
            .iter()
            .map(|s| s.to_lowercase())
            .collect();
        let exempt: std::collections::HashSet<String> = NOT_YET_OBSERVABLE
            .iter()
            .map(|(t, _)| t.to_lowercase())
            .collect();

        for tab in &catalog.tabs {
            let name = tab.name.to_lowercase();
            assert!(
                observable.contains(&name) || exempt.contains(&name),
                "tab {:?} ships in the app but the agent cannot read it: add an observe_app \
                 aspect, or add it to NOT_YET_OBSERVABLE with a reason. Everything in the app \
                 is meant to be queryable by the agent.",
                tab.name
            );
        }

        // An exemption for a tab that no longer exists, or for one that HAS
        // gained an aspect, is stale bookkeeping that hides real coverage.
        let tabs: std::collections::HashSet<String> =
            catalog.tabs.iter().map(|t| t.name.to_lowercase()).collect();
        for (tab, _) in NOT_YET_OBSERVABLE {
            let t = tab.to_lowercase();
            assert!(
                tabs.contains(&t),
                "NOT_YET_OBSERVABLE lists {tab:?}, which is not a shipped tab — remove it"
            );
            assert!(
                !observable.contains(&t),
                "{tab:?} now has an observe_app aspect — remove it from NOT_YET_OBSERVABLE"
            );
        }
    }

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
