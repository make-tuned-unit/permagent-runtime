//! Echo — the Watcher's *identity* (#672).
//!
//! The behavior lives in the daemon (`permagent-daemon`'s `proactive` module):
//! a gentle background loop over the Brain + the news around active projects.
//! This module is the *character* — its name and its self-knowledge — so Henry
//! knows the Watcher as one of his own, can describe it, and it takes its place
//! in the roster of background workers (Scheduler, Librarian, Steward,
//! Initiative). Identity lives in the `permagent` lib because that is where the
//! self-knowledge registry is; the daemon owns the runtime.

/// The Watcher's name — used in framing + self-knowledge. One source of truth.
pub const WATCHER_NAME: &str = "the Watcher";

/// Self-knowledge descriptor. Registered in
/// [`crate::agents::self_knowledge::WORKER_DESCRIPTORS`].
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "watcher",
        display_name: "The Watcher",
        category: crate::agents::self_knowledge::FeatureCategory::Worker,
        what_it_does:
            "A gentle background worker that watches for the single most useful thing to surface — \
             a dormant thread in the Brain (an entity woven through many memories, then gone quiet \
             while newer memories piled up elsewhere) or fresh news about a project the user is \
             actively working on — and reaches out at most once a day. It arrives as an in-app and \
             desktop notification, and (opt-in) a phone push. It only ever surfaces; it never acts, \
             and it stays silent when nothing clears the bar. Separately, once or twice a day \
             per project, it composes a short project insight from REAL per-project signals only \
             (never invented) onto the project overview — and briefs you about it, so you can \
             mention it without the user opening the panel",
        why_it_matters:
            "It is how the agent keeps the user on track without being asked — resurfacing \
             forgotten threads and catching news around their projects, the proactive presence a \
             permanent agent with a memory is uniquely able to offer. Because it reports to you \
             as well as to the panel, its observations reach the user through you rather than \
             waiting to be found. Gentle and rare by \
             construction: one nudge a day at most, quiet hours, deduped by subject and story, and \
             phone push stays off until the user opts in",
        // Workers are Queryable by contract; the Watcher has no live status to
        // merge yet, so it renders with no `_(now: …)_` suffix (worker_live_state
        // returns None for it) — same as the Scheduler with no jobs.
        state_source: crate::agents::self_knowledge::StateSource::Queryable,
        teaching: &[],
    };

/// Self-knowledge descriptor for the Watcher's project-insights card (the
/// daemon's `watcher_insights` loop, ruling 2026-07-28). The Watcher worker
/// above says it composes insights; this is the SURFACE where they land — the
/// Overview card on each project — so the agent can point the user at it.
/// Registered in [`crate::agents::self_knowledge::SURFACE_DESCRIPTORS`].
/// Static: editorial, no live claim.
pub const PROJECT_INSIGHTS_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "project_insights",
        display_name: "Project insights",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "A card on each project's Overview where the Watcher leaves a short, grounded \
             observation once or twice a day per project — composed only from REAL per-project \
             signals (notes added, kanban cards touched, stalled cards named by title, the \
             recorded stack, the site's 7-day pageviews) and never invented. There is no \
             notification and no badge; the user reads them as they browse the project, and \
             when a project has no signals or no model is available the card stays quiet rather \
             than filling with boilerplate",
        why_it_matters:
            "It is the Watcher's written trace inside the project itself — the place to point \
             the user when they ask what has moved on a project lately, and the reason you can \
             mention a stalled card by name. Silence over filler is the rule: an empty card \
             means nothing cleared the bar, not that the Watcher is broken",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };
