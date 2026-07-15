//! The tier ladder, task classification, and the escalation policy.
//!
//! This is the pure core of the cost-optimizing router: given a unit of work,
//! pick the *cheapest adequate* tier; when an attempt at that tier fails, walk
//! one rung up. It generalizes the orchestrator's roadmap-decomposition routing
//! (#249) — cheap local model first, escalate to the strong session provider on
//! failure/unparseable — so any caller can reuse the same policy.
//!
//! The ladder extends the existing worker cost vocabulary in
//! `goal_state::cost_tier_rank` (`local_free` < `subscription` < `paid_api`)
//! with a pooled tier BELOW local, per the mesh research. Cost order (this
//! enum's discriminants) and routing *eligibility* (the mesh gate in
//! `super::mesh`) are deliberately separate concerns: mesh is the cheapest tier,
//! but only ever eligible for gated batch work — never the interactive path.
//!
//! Deliberately free of async/DB/IO so it can be unit-tested without
//! infrastructure (mirrors `goal_state`).

/// A routing target, ordered cheapest-to-run first (the derived `Ord` follows
/// declaration order, so `MeshFree < LocalFree < CheapCloud < Frontier`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// A trusted, pooled/mesh endpoint — conceptually the cheapest, but only
    /// ever chosen for gated batch work (see `super::mesh`), never interactive.
    MeshFree,
    /// This machine's local model (Ollama) — free, always on-device.
    LocalFree,
    /// A cheap cloud model — for mechanical work a local model can't handle, and
    /// the reliable fallback when a free tier fails.
    CheapCloud,
    /// A frontier/strong model — reserved for hard reasoning.
    Frontier,
}

impl Tier {
    /// Cost rank, ascending (0 = cheapest). Mirrors and extends
    /// `goal_state::cost_tier_rank` with the pooled tier below local.
    pub fn cost_rank(&self) -> u8 {
        match self {
            Tier::MeshFree => 0,
            Tier::LocalFree => 1,
            Tier::CheapCloud => 2,
            Tier::Frontier => 3,
        }
    }

    /// Stable string label for logs/telemetry (not used to make decisions).
    /// Aligned with the existing `cost_tier` vocabulary where they overlap.
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::MeshFree => "mesh_free",
            Tier::LocalFree => "local_free",
            Tier::CheapCloud => "cheap_cloud",
            Tier::Frontier => "frontier",
        }
    }

    /// The next tier up (more capable, more expensive), or `None` at the top.
    pub fn escalated(&self) -> Option<Tier> {
        match self {
            Tier::MeshFree => Some(Tier::LocalFree),
            Tier::LocalFree => Some(Tier::CheapCloud),
            Tier::CheapCloud => Some(Tier::Frontier),
            Tier::Frontier => None,
        }
    }
}

/// What KIND of work a unit is — the axis that fixes the *minimum adequate*
/// tier. Hard reasoning needs a strong model; mechanical work does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskClass {
    /// Planning, multi-file changes, debugging — needs a frontier model.
    Reasoning,
    /// Apply-diff, summarize, grep-triage, structured extraction (e.g. roadmap
    /// decomposition), Reader/Librarian batch passes — a cheap/local model
    /// handles it, escalating only if it stumbles.
    Mechanical,
}

/// Coarse, caller-supplied signals about a unit of work. The router does not
/// parse free text — the caller (which already knows what it is dispatching)
/// sets the flags. Pure in, pure out.
///
/// Note on decomposition vs "planning": structured extraction like roadmap
/// decomposition (#249) is `mechanical` (cheap-first-then-escalate), whereas the
/// `planning` signal means open-ended architecture/design that needs a strong
/// model from the start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskSignals {
    /// Open-ended planning / architecture / design.
    pub planning: bool,
    /// Touches multiple files or needs cross-file reasoning.
    pub multi_file: bool,
    /// Debugging / root-causing a failure.
    pub debugging: bool,
    /// Mechanical: apply a diff, summarize, grep-triage, a Reader/Librarian
    /// pass, a structured extraction.
    pub mechanical: bool,
    /// Latency-tolerant background work (a prerequisite for the mesh path).
    pub batch: bool,
}

/// Classify a unit of work. Any hard-reasoning signal wins, even if `mechanical`
/// is also set: the expensive failure mode is a cheap model botching real
/// reasoning (wasted escalation round-trips), not a strong model doing simple
/// work, so we never *under*-provision a reasoning task.
pub fn classify(signals: &TaskSignals) -> TaskClass {
    if signals.planning || signals.multi_file || signals.debugging {
        TaskClass::Reasoning
    } else {
        TaskClass::Mechanical
    }
}

/// The minimum adequate STARTING tier for a task class (before the mesh gate).
/// Reasoning starts strong; mechanical starts on the local free tier and
/// escalates to cheap cloud if the local model stumbles.
pub fn minimum_tier(class: TaskClass) -> Tier {
    match class {
        TaskClass::Reasoning => Tier::Frontier,
        TaskClass::Mechanical => Tier::LocalFree,
    }
}

/// The outcome of attempting a unit of work at a tier — the signal that drives
/// escalation. Generalizes the orchestrator's `DecompositionError`
/// (`Provider` / `Unparseable`) so any caller can reuse the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    /// An acceptable result — done.
    Ok,
    /// The provider/endpoint call itself failed (unreachable, error, dropped).
    Failed,
    /// A response came back but could not be parsed/used even after a stricter
    /// retry — a quality-gate failure (mirrors `DecompositionError::Unparseable`).
    Unparseable,
    /// No response within the latency budget (the Reader's 60s Ollama timeout,
    /// generalized). For the mesh tier this triggers the cloud fallback.
    TimedOut,
}

/// What to do after an attempt at `current` yields an outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Accept the result — no escalation.
    Accept,
    /// Retry the same unit at a higher tier.
    Escalate(Tier),
    /// No higher tier exists — finalize as a failure (mirrors
    /// `finalize_decomposition` surfacing an unparseable/provider error rather
    /// than looping).
    GiveUp,
}

/// The escalation policy: accept on success; otherwise walk one tier up, and
/// give up at the ceiling.
///
/// The mesh tier is the one special case — a mesh failure/timeout degrades
/// DIRECTLY to a reliable cheap CLOUD model (never to local, which a lighter
/// device may not even be able to run the batch model on; never a hung retry;
/// never an abort), per the mesh research's auto-fallback requirement.
pub fn next_after(current: Tier, outcome: Attempt) -> Next {
    match outcome {
        Attempt::Ok => Next::Accept,
        Attempt::Failed | Attempt::Unparseable | Attempt::TimedOut => {
            if current == Tier::MeshFree {
                // The reliability wall: skip local, degrade straight to cloud.
                return Next::Escalate(super::mesh::fallback_tier());
            }
            match current.escalated() {
                Some(next) => Next::Escalate(next),
                None => Next::GiveUp,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_is_ordered_cheapest_first() {
        assert!(Tier::MeshFree < Tier::LocalFree);
        assert!(Tier::LocalFree < Tier::CheapCloud);
        assert!(Tier::CheapCloud < Tier::Frontier);
        assert_eq!(Tier::MeshFree.cost_rank(), 0);
        assert_eq!(Tier::Frontier.cost_rank(), 3);
        // Rank order agrees with Ord.
        let mut tiers = [
            Tier::Frontier,
            Tier::MeshFree,
            Tier::CheapCloud,
            Tier::LocalFree,
        ];
        tiers.sort();
        assert_eq!(
            tiers,
            [
                Tier::MeshFree,
                Tier::LocalFree,
                Tier::CheapCloud,
                Tier::Frontier
            ]
        );
    }

    #[test]
    fn escalated_walks_up_then_stops() {
        assert_eq!(Tier::MeshFree.escalated(), Some(Tier::LocalFree));
        assert_eq!(Tier::LocalFree.escalated(), Some(Tier::CheapCloud));
        assert_eq!(Tier::CheapCloud.escalated(), Some(Tier::Frontier));
        assert_eq!(Tier::Frontier.escalated(), None);
    }

    // ── Classification: task-class → tier (the test bar's first item) ──────

    #[test]
    fn reasoning_signals_classify_and_start_frontier() {
        for s in [
            TaskSignals {
                planning: true,
                ..Default::default()
            },
            TaskSignals {
                multi_file: true,
                ..Default::default()
            },
            TaskSignals {
                debugging: true,
                ..Default::default()
            },
        ] {
            assert_eq!(classify(&s), TaskClass::Reasoning);
            assert_eq!(minimum_tier(classify(&s)), Tier::Frontier);
        }
    }

    #[test]
    fn mechanical_work_classifies_and_starts_local() {
        let s = TaskSignals {
            mechanical: true,
            batch: true,
            ..Default::default()
        };
        assert_eq!(classify(&s), TaskClass::Mechanical);
        assert_eq!(minimum_tier(TaskClass::Mechanical), Tier::LocalFree);
    }

    #[test]
    fn reasoning_wins_over_mechanical_when_both_set() {
        // A multi-file change flagged mechanical is still reasoning — never
        // under-provision it to a cheap tier.
        let s = TaskSignals {
            mechanical: true,
            multi_file: true,
            ..Default::default()
        };
        assert_eq!(classify(&s), TaskClass::Reasoning);
    }

    #[test]
    fn empty_signals_default_to_mechanical() {
        assert_eq!(classify(&TaskSignals::default()), TaskClass::Mechanical);
    }

    // ── Escalation trigger: cheap failure → next tier (test bar item 2) ────

    #[test]
    fn success_is_accepted_never_escalated() {
        for t in [
            Tier::MeshFree,
            Tier::LocalFree,
            Tier::CheapCloud,
            Tier::Frontier,
        ] {
            assert_eq!(next_after(t, Attempt::Ok), Next::Accept);
        }
    }

    #[test]
    fn failure_escalates_one_tier() {
        assert_eq!(
            next_after(Tier::LocalFree, Attempt::Failed),
            Next::Escalate(Tier::CheapCloud)
        );
        assert_eq!(
            next_after(Tier::CheapCloud, Attempt::Unparseable),
            Next::Escalate(Tier::Frontier)
        );
    }

    #[test]
    fn frontier_failure_gives_up_rather_than_looping() {
        assert_eq!(next_after(Tier::Frontier, Attempt::Failed), Next::GiveUp);
        assert_eq!(
            next_after(Tier::Frontier, Attempt::Unparseable),
            Next::GiveUp
        );
        assert_eq!(next_after(Tier::Frontier, Attempt::TimedOut), Next::GiveUp);
    }

    #[test]
    fn mesh_failure_degrades_straight_to_cheap_cloud_not_local() {
        // The auto-fallback requirement: a dropped/slow mesh node → cheap cloud,
        // skipping local, never aborting.
        for o in [Attempt::Failed, Attempt::Unparseable, Attempt::TimedOut] {
            assert_eq!(
                next_after(Tier::MeshFree, o),
                Next::Escalate(Tier::CheapCloud)
            );
        }
    }
}
