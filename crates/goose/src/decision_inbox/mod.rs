//! Decision Inbox (Lane L3) — typed escalation, curation scoring, Henry's
//! Tier-1 policy flow, resume:auto, and Learn ingestion for Jesse's decisions.
//!
//! Part A shipped the seams (validation, scoring, memory formatting); Part B
//! wires them to Lane L1's decisions module: [`sink::SqlDecisionSink`]
//! persists escalations as decision rows, [`policy`] answers Tier-1 approvals
//! as `henry-policy` and resumes parked goals, [`curation`] feeds the `rank`
//! column, and [`learn`] turns jesse-answered decisions into Brain memories.

pub mod curation;
pub mod escalate;
pub mod learn;
pub mod policy;
pub mod sink;

/// Self-knowledge descriptor for the Decision Inbox surface (#353 standing
/// rule). Distinct from the downloads inbox (`crate::inbox`) — that name
/// collision confused the brief until 2026-07-10.
pub const DECISION_INBOX_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "decision_inbox",
        display_name: "Decision Inbox",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "The supervised-approval queue on the Home dashboard where decisions land for the \
             user: goal reviews with evidence, unblock requests when budgets exhaust, risk \
             gates, and initiative proposals — each approved or declined with one click",
        why_it_matters:
            "It is the human-judgment seam of the whole system — nothing destructive or \
             irreversible proceeds without passing through it. When you park a goal or propose \
             an initiative, this is where it surfaces; when the user wonders why something is \
             waiting, this is where to look",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[crate::agents::self_knowledge::TeachingStep {
            title: "Where decisions wait for you",
            body: "Open the Home dashboard's Decision Inbox. Anything the system needs the \
                   user's judgment on — goal approvals, unblocks, proposals — queues here \
                   with its evidence attached. Nothing risky happens without their say-so.",
            open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                tab: "Home",
                section: Some("decisions"),
            }),
            confirm: None,
        }],
    };
