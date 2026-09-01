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
    /// An earlier version of this test compared lowercased tab NAMES against
    /// `OBSERVABLE_SURFACES`. The shipped "Inbox" tab therefore matched the
    /// `inbox` surface by name while that aspect queried the Decision Inbox — a
    /// completely different store — and the test passed. The contract is now
    /// about the data source: each shipped tab declares what it `reads`, and
    /// `TAB_SURFACES` must name the same store for the aspect that claims it.
    ///
    /// This is the mirror of `every_teachable_surface_is_in_the_shipped_catalog`
    /// below: that one proves the agent never teaches a tab that does not exist;
    /// this one proves the app never ships a tab the agent cannot read.
    #[test]
    fn every_shipped_tab_is_observable_or_exempt() {
        use permagent::agents::platform_extensions::app_perception::{
            OBSERVABLE_SURFACES, TAB_SURFACES,
        };

        /// Tabs with no `observe_app` aspect yet, and why. SHRINK THIS LIST.
        /// An entry here is a promise not kept, not a design decision — the
        /// agent cannot answer questions about these surfaces from its own
        /// data, and will either say so or, worse, guess.
        const NOT_YET_OBSERVABLE: &[(&str, &str)] = &[];

        let catalog = crate::app_catalog::init();
        let exempt: std::collections::HashSet<String> = NOT_YET_OBSERVABLE
            .iter()
            .map(|(t, _)| t.to_lowercase())
            .collect();
        let shipped: std::collections::HashSet<String> =
            catalog.tabs.iter().map(|t| t.name.to_lowercase()).collect();

        let mut seen_tabs: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in TAB_SURFACES {
            let key = entry.tab.to_lowercase();
            assert!(
                seen_tabs.insert(key.clone()),
                "TAB_SURFACES lists tab {:?} more than once",
                entry.tab
            );
            assert!(
                shipped.contains(&key),
                "TAB_SURFACES names tab {:?}, which is not a shipped tab",
                entry.tab
            );
        }

        for tab in &catalog.tabs {
            let name = tab.name.to_lowercase();
            if exempt.contains(&name) {
                continue;
            }
            let matches: Vec<_> = TAB_SURFACES
                .iter()
                .filter(|e| e.tab.eq_ignore_ascii_case(&tab.name))
                .collect();
            assert!(
                matches.len() == 1,
                "tab {:?} ships in the app but the agent cannot read it: add an observe_app \
                 aspect, or add it to NOT_YET_OBSERVABLE with a reason. Everything in the app \
                 is meant to be queryable by the agent.",
                tab.name
            );
            let entry = matches[0];
            assert!(
                OBSERVABLE_SURFACES.contains(&entry.surface),
                "TAB_SURFACES maps {:?} to surface {:?}, which is not in OBSERVABLE_SURFACES",
                entry.tab,
                entry.surface
            );
            assert_eq!(
                entry.reads,
                tab.reads.as_str(),
                "aspect for tab {:?} claims to read {:?}, but the tab renders {:?} — the \
                 aspect reads a different store than the tab renders",
                tab.name,
                entry.reads,
                tab.reads
            );
        }

        // An exemption for a tab that no longer exists, or for one that HAS
        // gained an aspect, is stale bookkeeping that hides real coverage.
        for (tab, _) in NOT_YET_OBSERVABLE {
            let t = tab.to_lowercase();
            assert!(
                shipped.contains(&t),
                "NOT_YET_OBSERVABLE lists {tab:?}, which is not a shipped tab — remove it"
            );
            assert!(
                !TAB_SURFACES.iter().any(|e| e.tab.eq_ignore_ascii_case(tab)),
                "{tab:?} now has an observe_app aspect — remove it from NOT_YET_OBSERVABLE"
            );
        }
    }

    /// The panel-level half of the coverage guard above (#5 forensics).
    ///
    /// `every_shipped_tab_is_observable_or_exempt` only ever checked a tab's
    /// OWN top-level `reads` — it never looked at `panels`, a sub-section
    /// nested inside a tab that renders its own distinct store (e.g. the
    /// Documents panel inside the Projects tab's detail view). That gap is
    /// exactly how `project_documents` shipped invisible to the agent: the
    /// Projects tab looked fully covered (`reads: projects` ↔ `surface:
    /// projects`) while a whole nested store had no aspect naming it anywhere.
    ///
    /// Mirrors the tab-level test's shape: every catalog panel with a `reads`
    /// value must have exactly one `PANEL_SURFACES` entry, that entry's
    /// surface must be in `OBSERVABLE_SURFACES`, and its `reads` must match
    /// the catalog panel's `reads` verbatim (names are not evidence, same
    /// lesson as the tab-level guard's own history).
    #[test]
    fn every_shipped_panel_is_observable() {
        use permagent::agents::platform_extensions::app_perception::{
            OBSERVABLE_SURFACES, PANEL_SURFACES,
        };

        let catalog = crate::app_catalog::init();
        let shipped_tabs: std::collections::HashSet<String> =
            catalog.tabs.iter().map(|t| t.name.to_lowercase()).collect();

        // Reverse direction: every PANEL_SURFACES entry must name a real
        // shipped tab AND a real panel on it — a stale entry (tab renamed,
        // panel removed) would otherwise silently claim coverage of nothing.
        for entry in PANEL_SURFACES {
            assert!(
                shipped_tabs.contains(&entry.tab.to_lowercase()),
                "PANEL_SURFACES names tab {:?}, which is not a shipped tab",
                entry.tab
            );
            let tab = catalog
                .tabs
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(entry.tab))
                .expect("just proved this tab is shipped");
            assert!(
                tab.panels
                    .iter()
                    .any(|p| p.name.eq_ignore_ascii_case(entry.panel)),
                "PANEL_SURFACES names panel {:?} on tab {:?}, which the catalog does not list",
                entry.panel,
                entry.tab
            );
        }

        // Forward direction: every catalog panel must be covered.
        for tab in &catalog.tabs {
            for panel in &tab.panels {
                let matches: Vec<_> = PANEL_SURFACES
                    .iter()
                    .filter(|e| {
                        e.tab.eq_ignore_ascii_case(&tab.name)
                            && e.panel.eq_ignore_ascii_case(&panel.name)
                    })
                    .collect();
                assert!(
                    matches.len() == 1,
                    "panel {:?} ships inside tab {:?} but the agent cannot see its store \
                     ({:?}): add a PANEL_SURFACES entry mapping it to an observe_app aspect. \
                     Everything in the app is meant to be queryable by the agent — this is \
                     exactly how project_documents shipped unobservable.",
                    panel.name,
                    tab.name,
                    panel.reads
                );
                let entry = matches[0];
                assert!(
                    OBSERVABLE_SURFACES.contains(&entry.surface),
                    "PANEL_SURFACES maps {:?}/{:?} to surface {:?}, which is not in OBSERVABLE_SURFACES",
                    entry.tab,
                    entry.panel,
                    entry.surface
                );
                assert_eq!(
                    entry.reads,
                    panel.reads.as_str(),
                    "PANEL_SURFACES entry for {:?}/{:?} claims to read {:?}, but the catalog \
                     panel reads {:?} — the aspect reads a different store than the panel renders",
                    tab.name,
                    panel.name,
                    entry.reads,
                    panel.reads
                );
            }
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
