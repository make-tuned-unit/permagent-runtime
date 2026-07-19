//! Teachable curriculum — the "learn next" half of agent-led onboarding.
//!
//! [`usage`](super::usage) knows what the user has engaged. This module is the
//! curated set of capabilities the agent can actively *teach* — each mapped to
//! a **real, navigable surface** (a tab in the app catalog the agent can open
//! via `navigate_app`). `inventory − used = unused` is computed over this set
//! ([`learn_next`]) and ranked by the authoring order below (most valuable
//! onboarding first).
//!
//! ## The "no fake lesson" contract
//!
//! Every entry MUST satisfy two invariants, asserted by tests:
//! 1. `id` resolves to a live [`FeatureDescriptor`](super::FeatureDescriptor)
//!    via [`super::find_descriptor`] — so the lesson's *content*
//!    (`what_it_does` / `why_it_matters`) is real, not invented.
//! 2. `surface.tab` is a real app-catalog tab — checked here against
//!    [`NAV_CATALOG_TABS`], and cross-checked against the actually-shipped
//!    `catalog.yaml` by a goose-server test — so the lesson's *navigation*
//!    lands on a mounted view, not a dead tab.
//!
//! Reader-style lessons that teach a gesture (drag-and-drop) rather than a tab
//! are intentionally *not* here; the teaching tool still delivers them from
//! their authored steps. This table is specifically the navigable set that
//! drives the ranked learn-next list.

use chrono::{DateTime, Utc};

use super::SurfaceRef;

/// One teachable capability: a real descriptor id + the real surface to open.
#[derive(Debug, Clone, Copy)]
pub struct TeachableFeature {
    /// A live self-knowledge descriptor id.
    pub id: &'static str,
    /// The app-catalog surface the agent opens when teaching it.
    pub surface: SurfaceRef,
}

/// App-catalog tabs that are real, mounted, and agent-navigable (each maps to a
/// valid frontend `tool_type` / overlay). Kept in sync with
/// `goose-server/.../catalog.yaml`; a goose-server test asserts every
/// [`TEACHABLE`] tab is in the *shipped* catalog, so this list can never drift
/// into claiming a tab the app doesn't mount.
pub const NAV_CATALOG_TABS: &[&str] = &[
    "Build",
    "Automate",
    "Brain",
    "World",
    "Settings",
    "Projects",
    "Dashboard",
    "Grow",
    "Inbox",
];

/// The teachable curriculum, in value / onboarding order. Each id is a real
/// descriptor; each tab is in [`NAV_CATALOG_TABS`].
pub static TEACHABLE: &[TeachableFeature] = &[
    TeachableFeature {
        id: "brain",
        surface: SurfaceRef {
            tab: "Brain",
            section: None,
        },
    },
    TeachableFeature {
        id: "projects",
        surface: SurfaceRef {
            tab: "Projects",
            section: None,
        },
    },
    TeachableFeature {
        id: "build",
        surface: SurfaceRef {
            tab: "Build",
            section: None,
        },
    },
    TeachableFeature {
        id: "scheduler",
        surface: SurfaceRef {
            tab: "Automate",
            section: None,
        },
    },
    TeachableFeature {
        id: "decision_inbox",
        surface: SurfaceRef {
            tab: "Dashboard",
            section: Some("decisions"),
        },
    },
    TeachableFeature {
        id: "persona",
        surface: SurfaceRef {
            tab: "Settings",
            section: Some("identity"),
        },
    },
    TeachableFeature {
        id: "run_roster",
        surface: SurfaceRef {
            tab: "Automate",
            section: None,
        },
    },
    TeachableFeature {
        id: "grow",
        surface: SurfaceRef {
            tab: "Grow",
            section: None,
        },
    },
    TeachableFeature {
        id: "voice",
        surface: SurfaceRef {
            tab: "Settings",
            section: None,
        },
    },
    TeachableFeature {
        id: "web_search",
        surface: SurfaceRef {
            tab: "Settings",
            section: None,
        },
    },
    TeachableFeature {
        id: "devices",
        surface: SurfaceRef {
            tab: "Settings",
            section: Some("devices"),
        },
    },
    TeachableFeature {
        id: "inbox",
        surface: SurfaceRef {
            tab: "Inbox",
            section: None,
        },
    },
    TeachableFeature {
        id: "world_view",
        surface: SurfaceRef {
            tab: "World",
            section: None,
        },
    },
];

/// Find a teachable feature by id.
pub fn find_teachable(id: &str) -> Option<&'static TeachableFeature> {
    TEACHABLE.iter().find(|t| t.id == id)
}

/// All teachable capability ids, in curriculum order.
pub fn teachable_ids() -> Vec<&'static str> {
    TEACHABLE.iter().map(|t| t.id).collect()
}

/// `inventory − used`: the teachable capabilities the user has not engaged yet
/// (neither used nor taught), in value order. Reads the live global usage
/// store. The first element is the top thing to teach next.
pub fn learn_next() -> Vec<&'static TeachableFeature> {
    TEACHABLE
        .iter()
        .filter(|t| !super::usage::is_engaged(t.id))
        .collect()
}

/// Same computation against an explicit engaged-set — the pure core of
/// [`learn_next`], for tests and callers that already hold a usage snapshot.
pub fn learn_next_given(is_engaged: impl Fn(&str) -> bool) -> Vec<&'static TeachableFeature> {
    TEACHABLE.iter().filter(|t| !is_engaged(t.id)).collect()
}

// ── Proactive offer (gentle, rate-limited) ───────────────────────────────────

/// Config cursor: RFC3339 timestamp of the last time a proactive learn-next
/// hint was injected. Reuses the plain-config cursor pattern (like the
/// initiative driver) — no schema.
pub const LEARN_NEXT_LAST_OFFERED_KEY: &str = "learn_next_last_offered";

/// Minimum spacing between proactive learn-next offers. Gentle by construction —
/// at most one a day, echoing the Watcher's once-a-day discipline (this is the
/// same restraint, delivered through the existing in-context prompt seam rather
/// than a new notification channel).
const OFFER_COOLDOWN_HOURS: i64 = 24;

/// Pure cadence gate: offer a learn-next hint only when there is something
/// unused AND the cooldown since the last offer has elapsed (or there was none).
/// Pure and time-injected so the restraint is unit-testable.
pub fn should_offer_learn_next(
    now: DateTime<Utc>,
    last_offered: Option<DateTime<Utc>>,
    has_unused: bool,
) -> bool {
    if !has_unused {
        return false;
    }
    match last_offered {
        None => true,
        Some(prev) => now.signed_duration_since(prev).num_hours() >= OFFER_COOLDOWN_HOURS,
    }
}

/// If the cadence gate allows, pick the top unused capability and return an
/// agent-facing instruction to *gently* offer to show it (and record the offer
/// so it won't re-fire for a day). Returns `None` when nothing is unused, the
/// cooldown has not elapsed, or there is no descriptor. Impure (reads/writes the
/// config cursor + the live usage store); the pure decision is
/// [`should_offer_learn_next`].
pub fn proactive_learn_next_hint() -> Option<String> {
    let now = Utc::now();
    let last_offered = crate::config::Config::global()
        .get_param::<String>(LEARN_NEXT_LAST_OFFERED_KEY)
        .ok()
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let next = learn_next();
    if !should_offer_learn_next(now, last_offered, !next.is_empty()) {
        return None;
    }
    let top = next.first()?;
    let d = super::find_descriptor(top.id)?;

    // Record that we offered now (best-effort) so we don't re-inject for a day.
    if let Err(e) =
        crate::config::Config::global().set_param(LEARN_NEXT_LAST_OFFERED_KEY, now.to_rfc3339())
    {
        tracing::warn!(target: "onboarding", "failed to persist learn-next offer cursor: {e}");
    }

    Some(format!(
        "Onboarding nudge (once — never nag): the user has not tried **{name}** yet. {what}. \
         {why}. If it fits naturally in this conversation and you have not already raised it, \
         you may gently offer to show them — e.g. \"By the way, you haven't used {name} yet; \
         want me to walk you through it?\" If they say yes, call `load_feature_lesson` with \
         feature_id \"{id}\". If now is not a good moment, simply skip it — no pressure.",
        name = d.display_name,
        what = d.what_it_does,
        why = d.why_it_matters,
        id = top.id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_teachable_id_is_a_real_descriptor() {
        // Invariant 1: no fake lesson content.
        for t in TEACHABLE {
            assert!(
                super::super::find_descriptor(t.id).is_some(),
                "teachable id {:?} has no self-knowledge descriptor",
                t.id
            );
        }
    }

    #[test]
    fn every_teachable_tab_is_a_known_catalog_tab() {
        // Invariant 2 (local half): no dead-tab navigation. The goose-server
        // suite additionally cross-checks against the shipped catalog.yaml.
        let tabs: HashSet<&str> = NAV_CATALOG_TABS.iter().copied().collect();
        for t in TEACHABLE {
            assert!(
                tabs.contains(t.surface.tab),
                "teachable id {:?} opens tab {:?} which is not in NAV_CATALOG_TABS",
                t.id,
                t.surface.tab
            );
        }
    }

    #[test]
    fn teachable_ids_are_unique() {
        let mut seen = HashSet::new();
        for t in TEACHABLE {
            assert!(seen.insert(t.id), "duplicate teachable id {:?}", t.id);
        }
    }

    #[test]
    fn learn_next_excludes_engaged_and_preserves_value_order() {
        // Nothing engaged → the whole curriculum, in order.
        let all = learn_next_given(|_| false);
        assert_eq!(all.len(), TEACHABLE.len());
        assert_eq!(all[0].id, "brain");

        // Engage the top two → they drop out, order preserved.
        let engaged: HashSet<&str> = ["brain", "projects"].into_iter().collect();
        let next = learn_next_given(|id| engaged.contains(id));
        assert_eq!(next.len(), TEACHABLE.len() - 2);
        assert!(!next.iter().any(|t| t.id == "brain" || t.id == "projects"));
        // "build" was third → now the top recommendation.
        assert_eq!(next[0].id, "build");

        // Everything engaged → empty learn-next.
        assert!(learn_next_given(|_| true).is_empty());
    }

    #[test]
    fn offer_gate_respects_cooldown_and_unused() {
        use chrono::TimeZone;
        let now = Utc.with_ymd_and_hms(2026, 7, 16, 12, 0, 0).unwrap();

        // Nothing unused → never offer, regardless of cooldown.
        assert!(!super::should_offer_learn_next(now, None, false));

        // Unused + never offered before → offer.
        assert!(super::should_offer_learn_next(now, None, true));

        // Offered 23h ago → still cooling down.
        let recent = now - chrono::Duration::hours(23);
        assert!(!super::should_offer_learn_next(now, Some(recent), true));

        // Offered 25h ago → cooldown elapsed, offer again.
        let old = now - chrono::Duration::hours(25);
        assert!(super::should_offer_learn_next(now, Some(old), true));

        // Cooldown elapsed but nothing unused → still no offer.
        assert!(!super::should_offer_learn_next(now, Some(old), false));
    }
}
