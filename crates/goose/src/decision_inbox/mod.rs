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
/// rule). Distinct from the downloads inbox (`crate::inbox`, descriptor id
/// `downloads_inbox`); the Decision Inbox extension keeps its persisted config
/// name `inbox`, while the two surface ids remain unambiguous.
pub const DECISION_INBOX_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "decision_inbox",
        display_name: "Decision Inbox",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "The supervised-approval queue on the Home dashboard where decisions land for the \
             user: goal reviews with evidence, unblock requests when budgets exhaust, risk \
             gates, and initiative proposals — each approved, declined, or accepted-with-edits \
             in one click",
        why_it_matters:
            "It is the human-judgment seam of the whole system — nothing destructive or \
             irreversible proceeds without passing through it. When you park a goal or propose \
             an initiative, this is where it surfaces; when the user wonders why something is \
             waiting, this is where to look. And when the user revises a draft before accepting \
             it, I learn the correction — so my next draft moves toward how they would write it",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Where decisions wait for you",
                body: "Open the Home dashboard's Decision Inbox. Anything the system needs the \
                       user's judgment on — goal approvals, unblocks, proposals — queues here \
                       with its evidence attached. Nothing risky happens without their say-so.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Dashboard",
                    section: Some("decisions"),
                }),
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Approve with edits — teach me by revising",
                body: "When a proposal or draft is not quite right, do not just approve or \
                       decline it — revise the text first, then accept. I keep what the user \
                       accepted AND learn the change itself, so the next time I draft something \
                       similar it starts closer to how they would write it.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Dashboard",
                    section: Some("decisions"),
                }),
                confirm: None,
            },
        ],
    };
