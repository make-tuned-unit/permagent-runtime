//! Execution-grounded best-of-N — the accuracy test-time-compute lever.
//!
//! The single biggest raw-accuracy move for a cheap model is not a better
//! prompt or a neural reranker: it is to SAMPLE several candidate solutions and
//! SELECT the one that actually passes the tests (CodeT's dual-execution
//! agreement, +18.8pp pass@1; the DeepSWE self-consistency win). The verifier is
//! free and already ours (the `developer::verify` tool),
//! so selection is grounded in EXECUTION, not a learned score — the whole point,
//! and the part every frontier result agrees is where the accuracy actually
//! comes from.
//!
//! This module is the DECISION + ORCHESTRATION core of that mechanism, kept pure
//! and near-pure exactly like the rest of [`super`] (the test bar demands
//! testable decisions; CI is the build gate). It answers three questions and
//! nothing else:
//!
//! 1. **Should we even sample N?** ([`difficulty`] → [`plan_candidates`]) — the
//!    DIFFICULTY ROUTER. Best-of-N amplifies a cheap model on HARD tasks; on an
//!    easy edit N=1 is both cheaper and just as good, so we NEVER pay for N there.
//!    "Hard" is a concrete signal, not a guess: the task already failed `verify`
//!    at least once, or the caller flagged it hard up front.
//! 2. **How many, and may we afford it?** ([`candidate_count`], the COST GATE) —
//!    N is bounded ([`MAX_BEST_OF_N`]), CHEAP-TIER ONLY (never the frontier — see
//!    the caveat below), and collapses to 1 when the spend cap says so, so a
//!    sampling burst can never blow the budget.
//! 3. **Which candidate wins?** ([`select`]) — by RUNNING THE TESTS. Each
//!    candidate carries its `verify` verdict; the passer is selected. When several
//!    pass, CodeT-style dual-execution agreement breaks the tie: cluster the
//!    passers by which generated tests they pass and take the largest consensus
//!    cluster (lowest index within it, so the choice is deterministic). When none
//!    pass, the caller falls through to the normal fix-loop / verifier-driven tier
//!    escalation ([`super::escalation`], #742).
//!
//! The effectful fan-out — create N isolated worktrees, run a cheap-tier attempt
//! in each, run `verify` in each — is orchestrated by [`run_best_of_n`] over the
//! [`CandidateSource`] seam, so the parallel-candidate + isolated-verify wiring is
//! unit-testable with mocks (no worktrees, providers, or network). This mirrors
//! how #739 shipped the escalation DECISION core ([`super::escalation::decide_escalation`])
//! ahead of the live re-dispatch that #742 wired into the
//! `agents::platform_extensions::orchestrator`: the concrete
//! [`CandidateSource`] (a git worktree off the goal baseline → a cheap-tier
//! `summon` delegate into it → `VerifyTool::verify_with_cwd` on that worktree)
//! is the follow-up increment, dropped in behind this trait.
//!
//! ## Frontier caveats made structural (not advice)
//!
//! - **Select by EXECUTION, never a neural reranker.** [`select`] reads only
//!   `verify` verdicts and test-pass signatures — running the tests is the ~19pp
//!   win; a reranker buys ~2pp and is deliberately absent.
//! - **The difficulty router gates it.** [`candidate_count`] returns 1 for an easy
//!   task, so sample diversity is only ever paid for where it pays off.
//! - **Do NOT collapse sample diversity.** The driver dispatches N INDEPENDENT
//!   attempts (distinct isolated worktrees); nothing shares state, so the samples
//!   stay diverse — that diversity is what a passer is selected FROM.
//! - **Cost-gate, cheap-tier only, never the frontier.** N is bounded and only
//!   ever runs on the cheap tiers ($0 local / cheap cloud); the frontier tier
//!   short-circuits to N=1, and the spend cap collapses N to 1 before it can be
//!   crossed.

use async_trait::async_trait;

use super::budget::BudgetVerdict;
use super::tier::Tier;

/// Default number of candidates sampled for a task the difficulty router judged
/// HARD, before the cost gate clamps it. Four independent cheap attempts is the
/// sweet spot in the best-of-N curves (most of the pass@k gain, little waste).
/// CONFIGURABLE via [`KEY_BEST_OF_N`].
pub const BEST_OF_N_DEFAULT: u8 = 4;

/// Hard ceiling on N regardless of config — a sampling burst can never fan out
/// beyond this many parallel candidates (bounds cost AND local/worktree pressure).
pub const MAX_BEST_OF_N: u8 = 8;

/// Config key overriding [`BEST_OF_N_DEFAULT`].
pub const KEY_BEST_OF_N: &str = "PERMAGENT_BEST_OF_N";

/// Config key for the opt-in feature flag ([`best_of_n_enabled`]). Default OFF —
/// the mechanism is build-flag-gated until its live [`CandidateSource`] adapter
/// and eval land (mirrors the Librarian-atoms rollout discipline).
pub const KEY_BEST_OF_N_ENABLED: &str = "PERMAGENT_BEST_OF_N_ENABLED";

// ── The difficulty router ────────────────────────────────────────────────────

/// How hard a bounded edit/task looks — the gate that decides whether paying for
/// N>1 candidates is worth it. Best-of-N amplifies a cheap model on HARD tasks;
/// on easy ones N=1 is cheaper and just as accurate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    /// A routine edit expected to pass first try — never sample (N=1).
    Easy,
    /// Known-hard — sampling N>1 is warranted.
    Hard,
}

/// Coarse, caller-supplied signals about how hard a bounded task is. The router
/// does not parse free text — the caller (which already knows what it dispatched
/// and whether the last `verify` failed) sets the flags. Pure in, pure out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DifficultySignals {
    /// The task already failed `verify` at least once at this tier — the
    /// strongest cheap signal that one-shot editing is not landing it.
    pub failed_verify: bool,
    /// The caller flagged the task hard up front (e.g. multi-file / debugging,
    /// the reasoning class from [`super::tier::classify`]).
    pub flagged_hard: bool,
}

/// Classify a bounded task's difficulty. Any hard signal makes it [`Difficulty::Hard`];
/// with nothing flagged it is [`Difficulty::Easy`] (the common case, N=1).
pub fn difficulty(signals: &DifficultySignals) -> Difficulty {
    if signals.failed_verify || signals.flagged_hard {
        Difficulty::Hard
    } else {
        Difficulty::Easy
    }
}

// ── The cost gate + the plan ─────────────────────────────────────────────────

/// The concrete plan for one bounded task: how many candidates to sample and on
/// which (always cheap) tier. `n == 1` means "do NOT sample" — run the normal
/// single edit; `n > 1` means fan out N isolated candidates and select by verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BestOfNPlan {
    /// Candidates to sample. `1` ⇒ no sampling (the normal single-edit path).
    pub n: u8,
    /// The cheap tier every candidate runs on. Never [`Tier::Frontier`].
    pub tier: Tier,
}

impl BestOfNPlan {
    /// Whether this plan actually samples (N > 1). When false the caller skips
    /// best-of-N entirely and takes its normal single-edit path.
    pub fn samples(&self) -> bool {
        self.n > 1
    }
}

/// Build the best-of-N plan for a bounded task (pure). Combines the difficulty
/// verdict, the tier the work runs on, the requested N (usually [`load_best_of_n`]),
/// and the live spend verdict into a concrete, budget-safe candidate count.
pub fn plan_candidates(
    difficulty: Difficulty,
    tier: Tier,
    requested_n: u8,
    budget: BudgetVerdict,
) -> BestOfNPlan {
    BestOfNPlan {
        n: candidate_count(difficulty, tier, requested_n, budget),
        tier,
    }
}

/// The pure cost gate: how many candidates to actually sample, checked in
/// precedence order so the *cheapest-safe* count always wins.
///
/// 1. **Easy ⇒ 1.** Never pay for sampling on a routine edit.
/// 2. **Frontier ⇒ 1.** Best-of-N amplifies CHEAP models; N never runs on the
///    frontier tier (its cost, and the fact a frontier model rarely needs it).
/// 3. **Hard spend-stop ⇒ 1.** The hard ceiling halts everything — no fan-out.
/// 4. **Spend gate on a PAID tier ⇒ 1.** At the gate we add no gated spend, so a
///    cheap-CLOUD burst collapses to one. A FREE tier ($0 local/mesh) still
///    amplifies — that is exactly when the free amplifier matters most.
/// 5. Otherwise sample the requested N, clamped to `[2, MAX_BEST_OF_N]` (a task
///    we have decided to sample always gets at least a pair to select between).
fn candidate_count(
    difficulty: Difficulty,
    tier: Tier,
    requested_n: u8,
    budget: BudgetVerdict,
) -> u8 {
    if difficulty == Difficulty::Easy {
        return 1;
    }
    if tier == Tier::Frontier {
        return 1;
    }
    if budget.must_stop() {
        return 1;
    }
    if budget.needs_gate() && tier_costs_money(tier) {
        return 1;
    }
    requested_n.clamp(2, MAX_BEST_OF_N)
}

/// Whether a tier bills real money (cheap cloud and up) vs is free ($0 local /
/// mesh). Used by the cost gate so the spend gate only clamps paid sampling.
fn tier_costs_money(tier: Tier) -> bool {
    tier.cost_rank() >= Tier::CheapCloud.cost_rank()
}

// ── Execution-grounded selection (CodeT dual-execution agreement) ────────────

/// One candidate's execution verdict from running `verify` in its isolated
/// worktree. `Pass` ⇔ the verify tool returned a clean (non-error) result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// `verify` passed — this candidate is a selection eligible.
    Pass,
    /// `verify` failed — this candidate is discarded.
    Fail,
}

/// One sampled candidate's outcome: its dispatch index (the deterministic
/// tie-break key), whether `verify` passed, and — when the caller also ran a set
/// of generated tests — a fingerprint of WHICH tests passed, enabling CodeT
/// consensus clustering among passers. `None` signature ⇒ the plain "first
/// passer" fallback (the whole `verify` suite is the single test).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateVerdict {
    /// Dispatch order (0-based). The lowest index is the deterministic tie-break.
    pub index: usize,
    /// Whether `verify` passed for this candidate.
    pub verdict: Verdict,
    /// A fingerprint of the passed-test set (for CodeT clustering), or `None`.
    pub test_signature: Option<u64>,
}

/// The outcome of selecting among sampled candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// A candidate passed `verify` and was selected by execution.
    Selected {
        /// The chosen candidate's dispatch index.
        index: usize,
        /// How many passers shared its consensus cluster (1 when signatures are
        /// absent or unique) — the CodeT agreement size.
        agree: usize,
        /// Total candidates that passed `verify`.
        passers: usize,
    },
    /// No candidate passed `verify` — the caller falls through to the normal
    /// fix-loop / verifier-driven tier escalation.
    NonePassed,
}

/// Select the winning candidate by EXECUTION (pure). The passers (verify == Pass)
/// are the only eligibles; among them CodeT dual-execution agreement prefers the
/// LARGEST consensus cluster — the solution whose test outcomes the most other
/// solutions agree with — breaking ties toward the lowest dispatch index so the
/// choice is fully deterministic. With no passers the result is
/// [`Selection::NonePassed`] (the fall-through signal).
pub fn select(candidates: &[CandidateVerdict]) -> Selection {
    // Passers in dispatch order — the index is the deterministic tie-break key.
    let passers: Vec<CandidateVerdict> = candidates
        .iter()
        .copied()
        .filter(|c| c.verdict == Verdict::Pass)
        .collect();
    if passers.is_empty() {
        return Selection::NonePassed;
    }

    // A candidate's consensus size: how many passers share its test signature. A
    // passer with no signature is its own singleton (agreement = 1) — the
    // first-passer fallback then reduces to "lowest index".
    let cluster_size = |c: &CandidateVerdict| -> usize {
        match c.test_signature {
            Some(sig) => passers
                .iter()
                .filter(|p| p.test_signature == Some(sig))
                .count(),
            None => 1,
        }
    };

    // Largest cluster wins; equal clusters break to the lowest index (Reverse, so
    // `max_by_key` prefers the smaller index). Composite keys are unique (index is
    // unique), so the pick is deterministic regardless of iteration order.
    let best = passers
        .iter()
        .copied()
        .max_by_key(|c| (cluster_size(c), std::cmp::Reverse(c.index)))
        .expect("passers is non-empty");

    Selection::Selected {
        index: best.index,
        agree: cluster_size(&best),
        passers: passers.len(),
    }
}

// ── Orchestration over the effectful seam (mock-testable) ────────────────────

/// The result of a best-of-N run — the driver's report to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BestOfNOutcome {
    /// The plan did not sample (`n == 1`) — the caller runs its normal single
    /// edit. NOT a failure.
    Skipped,
    /// A candidate passed `verify` and was selected — the caller adopts its
    /// worktree/diff.
    Selected {
        /// The chosen candidate's dispatch index.
        index: usize,
        /// CodeT agreement size of the chosen cluster.
        agree: usize,
        /// How many candidates passed `verify`.
        passers: usize,
        /// How many candidates were sampled in total.
        sampled: usize,
    },
    /// N candidates ran but NONE passed `verify` — fall through to the normal
    /// fix-loop / verifier-driven tier escalation (#742).
    NonePassed {
        /// How many candidates were sampled.
        sampled: usize,
    },
}

/// Produces and verifies ONE candidate solution in ISOLATION — the effectful seam
/// the pure driver orchestrates. The concrete implementation (the follow-up
/// increment) creates a git worktree off the goal baseline, runs a cheap-tier
/// `summon` attempt in it, then runs the `verify` tool in that worktree and maps
/// its structured pass/fail to a [`CandidateVerdict`]. It is a trait so the
/// fan-out + selection is unit-testable with mocks — no worktrees, providers, or
/// network in the pure suite.
#[async_trait]
pub trait CandidateSource: Sync {
    /// Attempt candidate `index` in ISOLATION and return its execution verdict.
    /// Implementations MUST NOT share mutable state between candidates — the
    /// isolation is the whole point (one candidate's edits must never affect
    /// another's `verify`), and it is also what keeps the samples diverse.
    async fn attempt(&self, index: usize) -> CandidateVerdict;
}

/// Run best-of-N: fan out `plan.n` INDEPENDENT candidate attempts concurrently,
/// then [`select`] the passer by execution. A non-sampling plan (`n <= 1`)
/// short-circuits to [`BestOfNOutcome::Skipped`] without touching `source`, so an
/// easy or budget-clamped task pays nothing.
///
/// The driver owns dispatch order: `join_all` preserves it, and indices are
/// re-stamped from it, so selection is deterministic no matter what a source
/// reports for its own index.
pub async fn run_best_of_n<S>(plan: &BestOfNPlan, source: &S) -> BestOfNOutcome
where
    S: CandidateSource + ?Sized,
{
    if !plan.samples() {
        return BestOfNOutcome::Skipped;
    }
    let n = plan.n as usize;

    // Fan out N isolated attempts concurrently; each runs in its own worktree, so
    // they proceed in parallel and cannot contaminate one another.
    let attempts = (0..n).map(|i| source.attempt(i));
    let mut verdicts: Vec<CandidateVerdict> = futures::future::join_all(attempts).await;

    // Re-stamp indices from dispatch order (join_all is order-preserving) so the
    // deterministic tie-break holds regardless of what the source set.
    for (i, v) in verdicts.iter_mut().enumerate() {
        v.index = i;
    }

    match select(&verdicts) {
        Selection::Selected {
            index,
            agree,
            passers,
        } => BestOfNOutcome::Selected {
            index,
            agree,
            passers,
            sampled: n,
        },
        Selection::NonePassed => BestOfNOutcome::NonePassed { sampled: n },
    }
}

// ── Thin config wrappers (mirror `escalation::{max_escalations_from,load_*}`) ─

/// Apply an optional override over [`BEST_OF_N_DEFAULT`]. Split for pure testing
/// (mirrors the repo's pure-core-plus-thin-env-wrapper convention).
pub fn best_of_n_from(override_value: Option<u8>) -> u8 {
    override_value.unwrap_or(BEST_OF_N_DEFAULT)
}

/// Load the requested candidate count from config, falling back to the default.
pub fn load_best_of_n() -> u8 {
    let cfg = crate::config::Config::global();
    best_of_n_from(cfg.get_param::<u8>(KEY_BEST_OF_N).ok())
}

/// Whether execution-grounded best-of-N is enabled (opt-in; default OFF until the
/// live [`CandidateSource`] adapter and eval land).
pub fn best_of_n_enabled() -> bool {
    crate::config::Config::global()
        .get_param::<bool>(KEY_BEST_OF_N_ENABLED)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost_router::budget::{budget_verdict, BudgetConfig};

    // ── helpers ──────────────────────────────────────────────────────────────

    /// A spend verdict comfortably under every ceiling (no gate/stop).
    fn budget_ok() -> BudgetVerdict {
        budget_verdict(0.0, 0.0, &BudgetConfig::default())
    }

    /// A verdict sitting exactly at the session spend gate ($25 default).
    fn budget_at_gate() -> BudgetVerdict {
        let v = budget_verdict(0.0, 25.0, &BudgetConfig::default());
        assert!(v.needs_gate(), "test fixture must be at the gate");
        v
    }

    /// A verdict at the hard session ceiling ($50 default).
    fn budget_hard_stop() -> BudgetVerdict {
        let v = budget_verdict(0.0, 50.0, &BudgetConfig::default());
        assert!(v.must_stop(), "test fixture must be a hard stop");
        v
    }

    fn pass(index: usize, sig: Option<u64>) -> CandidateVerdict {
        CandidateVerdict {
            index,
            verdict: Verdict::Pass,
            test_signature: sig,
        }
    }

    fn fail(index: usize) -> CandidateVerdict {
        CandidateVerdict {
            index,
            verdict: Verdict::Fail,
            test_signature: None,
        }
    }

    // ── The difficulty router ────────────────────────────────────────────────

    #[test]
    fn no_signals_is_easy_and_a_failed_verify_or_flag_is_hard() {
        assert_eq!(difficulty(&DifficultySignals::default()), Difficulty::Easy);
        assert_eq!(
            difficulty(&DifficultySignals {
                failed_verify: true,
                ..Default::default()
            }),
            Difficulty::Hard,
            "a task that already failed verify is hard"
        );
        assert_eq!(
            difficulty(&DifficultySignals {
                flagged_hard: true,
                ..Default::default()
            }),
            Difficulty::Hard,
            "an up-front hard flag is hard"
        );
    }

    // ── The cost gate: easy → N=1 (never pay for sampling) ───────────────────

    #[test]
    fn easy_tasks_never_sample_regardless_of_tier_or_budget() {
        for tier in [Tier::MeshFree, Tier::LocalFree, Tier::CheapCloud] {
            let plan = plan_candidates(Difficulty::Easy, tier, BEST_OF_N_DEFAULT, budget_ok());
            assert_eq!(plan.n, 1, "easy on {tier:?} must be N=1");
            assert!(!plan.samples());
        }
    }

    #[test]
    fn hard_cheap_task_samples_the_requested_n() {
        let plan = plan_candidates(Difficulty::Hard, Tier::LocalFree, 4, budget_ok());
        assert_eq!(plan.n, 4);
        assert!(plan.samples());
        assert_eq!(plan.tier, Tier::LocalFree);
    }

    // ── The cost gate: never the frontier tier ───────────────────────────────

    #[test]
    fn frontier_tier_never_samples_even_when_hard() {
        // Best-of-N amplifies CHEAP models; N never runs on the frontier.
        let plan = plan_candidates(
            Difficulty::Hard,
            Tier::Frontier,
            BEST_OF_N_DEFAULT,
            budget_ok(),
        );
        assert_eq!(plan.n, 1, "frontier must short-circuit to N=1");
    }

    // ── The cost gate: spend cap ─────────────────────────────────────────────

    #[test]
    fn hard_spend_stop_collapses_n_to_one_on_every_tier() {
        for tier in [Tier::MeshFree, Tier::LocalFree, Tier::CheapCloud] {
            let plan = plan_candidates(
                Difficulty::Hard,
                tier,
                BEST_OF_N_DEFAULT,
                budget_hard_stop(),
            );
            assert_eq!(plan.n, 1, "a hard stop halts sampling on {tier:?}");
        }
    }

    #[test]
    fn spend_gate_collapses_paid_tier_but_free_tier_still_amplifies() {
        // At the gate we add no GATED spend, so a cheap-cloud burst collapses …
        let paid = plan_candidates(Difficulty::Hard, Tier::CheapCloud, 4, budget_at_gate());
        assert_eq!(paid.n, 1, "paid sampling collapses at the spend gate");

        // … but the $0 free tiers keep amplifying — that is when it matters most.
        for free in [Tier::MeshFree, Tier::LocalFree] {
            let plan = plan_candidates(Difficulty::Hard, free, 4, budget_at_gate());
            assert_eq!(plan.n, 4, "free tier {free:?} still samples at the gate");
        }
    }

    // ── The cost gate: N is bounded ──────────────────────────────────────────

    #[test]
    fn n_is_clamped_between_two_and_the_ceiling() {
        // A hard task we have decided to sample always gets at least a pair …
        let low = plan_candidates(Difficulty::Hard, Tier::LocalFree, 1, budget_ok());
        assert_eq!(low.n, 2, "a sampled task gets at least 2 candidates");
        let zero = plan_candidates(Difficulty::Hard, Tier::LocalFree, 0, budget_ok());
        assert_eq!(zero.n, 2);

        // … and never more than the hard ceiling, no matter what config asks for.
        let high = plan_candidates(Difficulty::Hard, Tier::LocalFree, 100, budget_ok());
        assert_eq!(high.n, MAX_BEST_OF_N, "N is bounded by the ceiling");
    }

    // ── Selection: pick the passer by execution ──────────────────────────────

    #[test]
    fn none_pass_is_the_fall_through_signal() {
        let candidates = [fail(0), fail(1), fail(2)];
        assert_eq!(select(&candidates), Selection::NonePassed);
        // The empty case is also a fall-through, never a panic.
        assert_eq!(select(&[]), Selection::NonePassed);
    }

    #[test]
    fn the_single_passer_is_selected() {
        let candidates = [fail(0), pass(1, None), fail(2)];
        assert_eq!(
            select(&candidates),
            Selection::Selected {
                index: 1,
                agree: 1,
                passers: 1,
            }
        );
    }

    #[test]
    fn multiple_passers_without_signatures_take_the_first_passer_deterministically() {
        let candidates = [fail(0), pass(1, None), pass(2, None), pass(3, None)];
        let got = select(&candidates);
        assert_eq!(
            got,
            Selection::Selected {
                index: 1,
                agree: 1,
                passers: 3,
            },
            "with no signatures the lowest-index passer wins"
        );
        // Deterministic: order of the input passers must not change the pick.
        let reordered = [pass(3, None), pass(2, None), pass(1, None), fail(0)];
        if let Selection::Selected { index, .. } = select(&reordered) {
            assert_eq!(index, 1, "the lowest index still wins after reordering");
        } else {
            panic!("expected a selection");
        }
    }

    #[test]
    fn codet_agreement_prefers_the_largest_consensus_cluster() {
        // Four passers: three agree on test-signature 0xAA, one is a lone 0xBB.
        // CodeT selects from the LARGEST cluster (the 0xAA trio), lowest index.
        let candidates = [
            pass(0, Some(0xBB)),
            pass(1, Some(0xAA)),
            pass(2, Some(0xAA)),
            pass(3, Some(0xAA)),
        ];
        assert_eq!(
            select(&candidates),
            Selection::Selected {
                index: 1,
                agree: 3,
                passers: 4,
            },
            "the 3-way consensus cluster wins over the lone passer even at index 0"
        );
    }

    #[test]
    fn equal_clusters_break_to_the_lowest_index() {
        // Two clusters of size two — the tie breaks to the lowest overall index.
        let candidates = [
            pass(0, Some(0xAA)),
            pass(1, Some(0xBB)),
            pass(2, Some(0xAA)),
            pass(3, Some(0xBB)),
        ];
        assert_eq!(
            select(&candidates),
            Selection::Selected {
                index: 0,
                agree: 2,
                passers: 4,
            }
        );
    }

    // ── Orchestration over the seam (mock-tested) ────────────────────────────

    /// A mock [`CandidateSource`]: per-index scripted verdicts, recording which
    /// indices were attempted (to prove the fan-out actually dispatched them all).
    struct MockSource {
        scripted: Vec<(Verdict, Option<u64>)>,
        attempted: std::sync::Mutex<Vec<usize>>,
    }

    impl MockSource {
        fn new(scripted: Vec<(Verdict, Option<u64>)>) -> Self {
            Self {
                scripted,
                attempted: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn attempted_indices(&self) -> Vec<usize> {
            let mut v = self.attempted.lock().unwrap().clone();
            v.sort_unstable();
            v
        }
    }

    #[async_trait::async_trait]
    impl CandidateSource for MockSource {
        async fn attempt(&self, index: usize) -> CandidateVerdict {
            self.attempted.lock().unwrap().push(index);
            let (verdict, sig) = self.scripted[index];
            // Deliberately report a WRONG index to prove the driver re-stamps it.
            CandidateVerdict {
                index: 999,
                verdict,
                test_signature: sig,
            }
        }
    }

    #[tokio::test]
    async fn a_non_sampling_plan_skips_without_touching_the_source() {
        let source = MockSource::new(vec![(Verdict::Pass, None)]);
        let plan = BestOfNPlan {
            n: 1,
            tier: Tier::LocalFree,
        };
        assert_eq!(run_best_of_n(&plan, &source).await, BestOfNOutcome::Skipped);
        assert!(
            source.attempted_indices().is_empty(),
            "a skipped plan must not attempt any candidate"
        );
    }

    #[tokio::test]
    async fn fans_out_all_candidates_and_selects_the_passer() {
        // Four candidates; two pass and agree on the same signature → that cluster
        // wins. All four indices must have been attempted (the fan-out is real).
        let source = MockSource::new(vec![
            (Verdict::Fail, None),
            (Verdict::Pass, Some(7)),
            (Verdict::Fail, None),
            (Verdict::Pass, Some(7)),
        ]);
        let plan = BestOfNPlan {
            n: 4,
            tier: Tier::LocalFree,
        };
        let outcome = run_best_of_n(&plan, &source).await;
        assert_eq!(
            outcome,
            BestOfNOutcome::Selected {
                index: 1,
                agree: 2,
                passers: 2,
                sampled: 4,
            }
        );
        assert_eq!(
            source.attempted_indices(),
            vec![0, 1, 2, 3],
            "every candidate must be dispatched (fan-out, not short-circuit)"
        );
    }

    #[tokio::test]
    async fn re_stamps_indices_from_dispatch_order() {
        // The mock lies about its index (always 999); the driver must re-stamp
        // from dispatch order, so the selected index is the true one (0 here).
        let source = MockSource::new(vec![(Verdict::Pass, None), (Verdict::Fail, None)]);
        let plan = BestOfNPlan {
            n: 2,
            tier: Tier::LocalFree,
        };
        assert_eq!(
            run_best_of_n(&plan, &source).await,
            BestOfNOutcome::Selected {
                index: 0,
                agree: 1,
                passers: 1,
                sampled: 2,
            }
        );
    }

    #[tokio::test]
    async fn none_passing_reports_fall_through_with_the_sample_count() {
        let source = MockSource::new(vec![
            (Verdict::Fail, None),
            (Verdict::Fail, None),
            (Verdict::Fail, None),
        ]);
        let plan = BestOfNPlan {
            n: 3,
            tier: Tier::CheapCloud,
        };
        assert_eq!(
            run_best_of_n(&plan, &source).await,
            BestOfNOutcome::NonePassed { sampled: 3 },
            "no passer → fall through to the fix-loop / escalation"
        );
        assert_eq!(source.attempted_indices(), vec![0, 1, 2]);
    }

    #[tokio::test]
    async fn works_through_a_dyn_source() {
        // The seam is object-safe: the real adapter can be handed in as `&dyn`.
        let source = MockSource::new(vec![(Verdict::Fail, None), (Verdict::Pass, None)]);
        let dyn_source: &dyn CandidateSource = &source;
        let plan = BestOfNPlan {
            n: 2,
            tier: Tier::LocalFree,
        };
        assert_eq!(
            run_best_of_n(&plan, dyn_source).await,
            BestOfNOutcome::Selected {
                index: 1,
                agree: 1,
                passers: 1,
                sampled: 2,
            }
        );
    }

    // ── Config wrappers ──────────────────────────────────────────────────────

    #[test]
    fn best_of_n_default_and_override() {
        assert_eq!(best_of_n_from(None), BEST_OF_N_DEFAULT);
        assert_eq!(best_of_n_from(Some(6)), 6);
    }
}
