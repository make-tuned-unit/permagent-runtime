//! Decision Inbox (Lane L3, Part A) — typed escalation, curation scoring,
//! and Learn ingestion for Jesse's decisions.
//!
//! Part A is deliberately self-contained: nothing here depends on the
//! decisions table / API (Lane L1). Persistence goes through the
//! [`escalate::DecisionSink`] seam; the real sink is wired in Part B.

pub mod curation;
pub mod escalate;
pub mod learn;
