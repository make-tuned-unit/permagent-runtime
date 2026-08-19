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
/// Cross-project pooled learning. It is what rescues the low-traffic projects:
/// one action on one small site is underpowered forever, while the same
/// strategy tried across every active project has a sample size worth reasoning
/// about. It segments before it pools — by traffic tier and site shape —
/// because a naive aggregate over projects that are not exchangeable produces
/// exactly the Simpson's paradox the proposal names, an aggregate reading
/// "helped" while quietly failing on the segment it is about to be applied to.
pub mod pooled;
pub mod power;
pub mod store;
pub mod sweep;

/// Self-knowledge descriptor for the measurement worker. The behavior is the
/// daemon's nightly pass (`growth_sweep.rs`, over [`sweep::run`]); the identity
/// lives here, in the lib half, because that is where the self-knowledge
/// registry is (the Echo/Watcher split). Registered in
/// [`crate::agents::self_knowledge::WORKER_DESCRIPTORS`].
///
/// Queryable by the worker contract, but there is no cheap live signal to
/// merge yet, so `worker_live_state_for` returns `None` and it renders
/// editorially — the same as the Watcher.
pub const GROWTH_MEASUREMENT_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "growth_measurement",
        display_name: "Growth measurement",
        category: crate::agents::self_knowledge::FeatureCategory::Worker,
        what_it_does:
            "A nightly measurement pass that closes the Grow loop — action, verify, measure, \
             learn. When a suggested growth action is verified as shipped, the target metric and \
             the direction it should move are pre-registered and the before-window is frozen; \
             then, as each 7-, 14- and 28-day window closes, the pass compares after with before \
             against the project's own week-to-week swing and records a verdict: helped, \
             hindered, no effect, inconclusive, or confounded when another action overlaps. \
             Inconclusive is a first-class outcome — at typical traffic it is the common one — \
             and every verdict carries the two numbers it rests on; when there is nothing to \
             judge it says nothing",
        why_it_matters:
            "It is what makes growth advice accountable instead of self-assessed prose: the \
             grade is computed from the project's own analytics events, never written by a \
             model. When the user asks whether something worked, read the verdict and its \
             numbers, say \"inconclusive\" plainly when that is the answer, and never say an \
             action caused a change — this is a before-and-after with an honesty gate, not an \
             experiment",
        state_source: crate::agents::self_knowledge::StateSource::Queryable,
        teaching: &[],
    };
