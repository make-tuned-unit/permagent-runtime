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
             and it stays silent when nothing clears the bar",
        why_it_matters:
            "It is how the agent keeps the user on track without being asked — resurfacing \
             forgotten threads and catching news around their projects, the proactive presence a \
             permanent agent with a memory is uniquely able to offer. Gentle and rare by \
             construction: one nudge a day at most, quiet hours, deduped by subject and story, and \
             phone push stays off until the user opts in",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };
