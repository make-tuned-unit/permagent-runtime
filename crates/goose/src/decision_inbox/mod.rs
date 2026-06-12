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
