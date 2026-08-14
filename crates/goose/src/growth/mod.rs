//! Grow — the action → verify → measure → learn loop.
//!
//! Ratified design: `docs/proposals/grow-action-outcome-loop.md`. This module is
//! the honest half: durable identity for a suggested action, closed measurement
//! windows, and the power check that decides whether a verdict is sayable at
//! all.
//!
//! The design's own framing of the risk, which every function here is shaped by:
//!
//! > A feature that says "this helped, +12%" off 40 pageviews is not measuring;
//! > it is pattern-matching noise and presenting it as evidence.
//!
//! So three rules are enforced in code, not in prose:
//!
//! * **Pre-registration.** `target_metric` and `target_dir` are written before a
//!   verdict is computed, and [`metrics::TargetMetric::parse`] rejects anything
//!   outside the allowlist — a metric chosen after the fact is unfalsifiable.
//! * **`inconclusive` is first-class.** [`power::judge`] reaches it whenever the
//!   observed delta sits inside the project's own week-to-week swing, which at
//!   `MIN_PAGEVIEWS = 20` traffic is the common case, not an error path.
//! * **Every verdict carries its numbers.** `rationale` is `NOT NULL` in the
//!   schema (spectral_schema.rs:2967) and every branch of `judge` populates it
//!   with the two values it rests on.
//!
//! The word "caused" is never used in a rationale — this is a before/after with
//! an honesty gate, not an experiment (proposal "Non-goals").

pub mod metrics;
pub mod power;
pub mod store;
pub mod sweep;
