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
    /// The model produced output and the call itself succeeded, but the project's
    /// OWN checks (the `verify` tool) still fail the SAME way after a bounded run
    /// of self-fix rounds — "the model can't fix it here," not "the call broke."
    /// Kept distinct from [`Attempt::Failed`] so telemetry can tell a capability
    /// gap from a transport failure; both escalate one tier. This is the signal a
    /// verifier-driven loop reports to climb to a stronger model — see
    /// [`VerifyEscalation`].
    VerifyFailed,
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
        Attempt::Failed | Attempt::Unparseable | Attempt::TimedOut | Attempt::VerifyFailed => {
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

/// The number of CONSECUTIVE same-normalized `verify` failures for one goal at
/// which a stronger model is warranted. Mirrors the runaway-loop guard's S6
/// verify-loop escalate threshold (`tool_monitor::S6_ESCALATE_AT`) so the two
/// halves of the loop — "stop this identical retry" and "climb to a stronger
/// model" — fire on the same evidence.
pub const VERIFY_ESCALATE_AT: u32 = 3;

/// What a verifier-driven fix loop should do next, given the observed run of
/// identical `verify` failures for one goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyEscalationAction {
    /// Below the same-failure threshold — keep letting the current model fix it.
    KeepFixing,
    /// The Nth identical failure — re-attempt the fix on this stronger tier.
    Escalate(Tier),
    /// The tier ceiling or the per-goal escalation budget is spent — stop and
    /// hand the goal to the human (never climb forever).
    Exhausted,
}

/// The verifier-driven escalation controller (#1): owns ONE goal's current tier
/// and a bounded escalation budget so a stuck `verify` loop climbs to a stronger
/// model ONCE PER OBSERVED SAME-FAILURE and no further.
///
/// It is the decision half of the closed verify loop: the runaway-loop guard's
/// S6 signal counts CONSECUTIVE identical `verify` failures across the edits
/// between them (`tool_monitor`), and this controller turns the Nth such failure
/// into a concrete stronger tier via [`next_after`]`(_, `[`Attempt::VerifyFailed`]`)`.
///
/// Two guardrails from the model-cascade literature are structural here, not
/// advisory — they prevent the documented "silently escalate everything to the
/// frontier" failure:
/// 1. **Consecutive-same-failure only.** Callers report [`Self::on_same_failure`]
///    with the run length of IDENTICAL failures; a changed/transient failure
///    resets that count upstream, so escalation never fires on varying output.
/// 2. **Bounded budget.** At most `max_escalations` climbs per goal regardless of
///    how many tiers remain, so a pathological goal cannot walk the whole ladder
///    to the frontier on repeated (but genuine) failures.
///
/// Pure and owns no I/O — the live re-dispatch that acts on an `Escalate` (swap
/// the goal worker's model and re-run the fix) is wired by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifyEscalation {
    current: Tier,
    escalations_used: u8,
    max_escalations: u8,
}

impl VerifyEscalation {
    /// Start a goal on `start` (the tier its worker was dispatched at) with a
    /// per-goal budget of `max_escalations` climbs.
    pub fn new(start: Tier, max_escalations: u8) -> Self {
        Self {
            current: start,
            escalations_used: 0,
            max_escalations,
        }
    }

    /// The tier the goal is currently attempting on.
    pub fn current(&self) -> Tier {
        self.current
    }

    /// Escalation climbs spent so far on this goal.
    pub fn escalations_used(&self) -> u8 {
        self.escalations_used
    }

    /// Report that `verify` has now failed identically `consecutive` times in a
    /// row for this goal (the runaway-loop guard's S6 count). Below the threshold
    /// this is [`VerifyEscalationAction::KeepFixing`]; at or above it, the tier
    /// advances one rung (recorded as the new `current`) until the budget or the
    /// ladder ceiling is reached, after which it is
    /// [`VerifyEscalationAction::Exhausted`].
    ///
    /// Idempotent within a threshold band is NOT assumed: the caller reports the
    /// running consecutive count, and this mutates only when it decides to climb,
    /// so calling it repeatedly at the same count past a climb yields `Exhausted`
    /// once the budget is spent rather than re-climbing.
    pub fn on_same_failure(&mut self, consecutive: u32) -> VerifyEscalationAction {
        if consecutive < VERIFY_ESCALATE_AT {
            return VerifyEscalationAction::KeepFixing;
        }
        if self.escalations_used >= self.max_escalations {
            return VerifyEscalationAction::Exhausted;
        }
        match next_after(self.current, Attempt::VerifyFailed) {
            Next::Escalate(next) => {
                self.current = next;
                self.escalations_used += 1;
                VerifyEscalationAction::Escalate(next)
            }
            // At the ceiling there is no stronger tier — stop, don't loop.
            Next::Accept | Next::GiveUp => VerifyEscalationAction::Exhausted,
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

    // ── Verifier-driven escalation (#1): the decision core ─────────────────

    #[test]
    fn verify_failed_escalates_one_tier_like_a_failure() {
        // A verify failure is a capability signal, routed exactly like a plain
        // failure: climb one rung, and give up (never loop) at the ceiling.
        assert_eq!(
            next_after(Tier::LocalFree, Attempt::VerifyFailed),
            Next::Escalate(Tier::CheapCloud)
        );
        assert_eq!(
            next_after(Tier::CheapCloud, Attempt::VerifyFailed),
            Next::Escalate(Tier::Frontier)
        );
        assert_eq!(
            next_after(Tier::Frontier, Attempt::VerifyFailed),
            Next::GiveUp
        );
        // Mesh degrades straight to cheap cloud on a verify failure too.
        assert_eq!(
            next_after(Tier::MeshFree, Attempt::VerifyFailed),
            Next::Escalate(Tier::CheapCloud)
        );
    }

    #[test]
    fn verify_escalation_keeps_fixing_below_the_threshold() {
        // Transient/varying failures never reach the consecutive-same threshold,
        // so the controller must NOT climb — it lets the current model keep
        // trying. This is the "don't escalate everything to frontier" guardrail.
        let mut esc = VerifyEscalation::new(Tier::LocalFree, 2);
        for n in 0..VERIFY_ESCALATE_AT {
            assert_eq!(
                esc.on_same_failure(n),
                VerifyEscalationAction::KeepFixing,
                "consecutive={n} is below threshold and must not escalate"
            );
        }
        assert_eq!(esc.current(), Tier::LocalFree, "no climb below threshold");
        assert_eq!(esc.escalations_used(), 0);
    }

    #[test]
    fn verify_escalation_selects_next_after_on_the_nth_same_failure() {
        // The Nth identical verify failure climbs exactly one rung — the tier the
        // fix is re-attempted on is `next_after(current, VerifyFailed)`.
        let mut esc = VerifyEscalation::new(Tier::LocalFree, 3);
        assert_eq!(
            esc.on_same_failure(VERIFY_ESCALATE_AT),
            VerifyEscalationAction::Escalate(Tier::CheapCloud)
        );
        assert_eq!(esc.current(), Tier::CheapCloud, "current advanced one rung");
        assert_eq!(esc.escalations_used(), 1);
    }

    #[test]
    fn verify_escalation_stops_at_the_max_escalations_budget() {
        // Budget = 1: the first same-failure climbs LocalFree → CheapCloud, but
        // the second is refused even though Frontier is still available — the
        // per-goal cap, not the ladder ceiling, stops it.
        let mut esc = VerifyEscalation::new(Tier::LocalFree, 1);
        assert_eq!(
            esc.on_same_failure(VERIFY_ESCALATE_AT),
            VerifyEscalationAction::Escalate(Tier::CheapCloud)
        );
        assert_eq!(
            esc.on_same_failure(VERIFY_ESCALATE_AT + 1),
            VerifyEscalationAction::Exhausted,
            "budget spent → stop, do NOT climb to the frontier"
        );
        assert_eq!(
            esc.current(),
            Tier::CheapCloud,
            "current must not advance past the budgeted stop"
        );
        assert_eq!(esc.escalations_used(), 1);
    }

    #[test]
    fn verify_escalation_stops_at_the_tier_ceiling() {
        // A generous budget still cannot climb past Frontier — the ladder ceiling
        // is the hard stop (there is no stronger model to escalate to).
        let mut esc = VerifyEscalation::new(Tier::CheapCloud, 9);
        assert_eq!(
            esc.on_same_failure(VERIFY_ESCALATE_AT),
            VerifyEscalationAction::Escalate(Tier::Frontier)
        );
        assert_eq!(
            esc.on_same_failure(VERIFY_ESCALATE_AT),
            VerifyEscalationAction::Exhausted,
            "at the frontier there is no stronger tier → stop, never loop"
        );
        assert_eq!(esc.current(), Tier::Frontier);
    }
}
