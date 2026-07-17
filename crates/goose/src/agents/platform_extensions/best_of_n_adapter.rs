//! The live best-of-N candidate adapter — actually RUN the sampling (#743's
//! decision brain, made to move).
//!
//! #743 shipped the pure brain: the difficulty router, the cost gate, and the
//! CodeT execution-selection ([`crate::cost_router::best_of_n`]). It orchestrates
//! candidates over a [`crate::cost_router::CandidateSource`] seam but ships only a
//! mock source, so nothing yet creates a worktree or runs a model. This module is
//! the ACTION half: it binds that seam to the live primitives so, on a hard /
//! repeated-`verify`-fail task, best-of-N genuinely samples N cheap-tier
//! candidates in isolation, verifies each, and selects the passer.
//!
//! ## The shape (mirrors the #739 → #742 escalation split)
//!
//! Candidate generation is dispatched through [`crate::agents::subagent_handler::run_subagent_task`]
//! (a `Send` future) and each candidate runs in its own detached git worktree —
//! so the WHOLE sampling can run in a detached `Send` task spawned from the
//! runaway-loop monitor, exactly like [`super::orchestrator::escalate_verify_fix_loop`]
//! (#742). The only `!Send` path — the orchestrator's own `dispatch_goal` — is
//! deliberately NOT used: candidates go through the summon path, which is `Send`.
//!
//! The orchestration is expressed over four effect SEAMS so the fan-out /
//! isolate / verify / select / apply / fall-through logic is unit-testable with
//! mocks (no worktrees, providers, or network):
//!
//! - [`CandidateWorktrees`] — create an isolated worktree off the goal baseline,
//!   and reap it. Real impl: [`GoalWorktrees`] over [`super::goal_engine`].
//! - [`CandidateGenerator`] — run one cheap-tier attempt inside a worktree.
//! - [`CandidateVerifier`] — run `verify` in a worktree. Real impl:
//!   [`VerifyToolVerifier`] over the `verify` tool.
//! - [`WinnerApplier`] — adopt the winning candidate's work into the goal.
//!
//! [`run_best_of_n_sampling`] ties them together: honor the feature flag
//! (`PERMAGENT_BEST_OF_N_ENABLED`, default OFF) → [`plan_candidates`][crate::cost_router::plan_candidates]
//! (difficulty router + cost gate: cheap-tier only, never the frontier,
//! spend-capped) → [`run_best_of_n`][crate::cost_router::run_best_of_n] (fan out N
//! isolated candidates, CodeT-select the passer) → apply the winner, or on
//! `NonePassed` fall through to the normal fix-loop / #742 escalation → reap every
//! candidate worktree.
//!
//! ## What is REAL here vs. the one flagged wiring step
//!
//! Real and CI-covered: the whole orchestration ([`run_best_of_n_sampling`] /
//! [`sample`]), the four seams, and the real [`GoalWorktrees`] + [`VerifyToolVerifier`].
//! Deferred to the follow-up (it needs a compiler this Intel-Mac dispatch box does
//! not have — the summon subagent assembly spans ~a dozen types across four
//! modules, and adopting the winner into the live goal state machine touches the
//! verification/worktree/push pipeline): the real [`CandidateGenerator`] (a
//! cheap-tier `run_subagent_task` in the worktree) and the real [`WinnerApplier`]
//! (splice the winner into the goal), plus the monitor spawn on the difficulty
//! trigger. All are gated behind the default-OFF flag, so nothing activates until
//! that step lands and is compile-verified on the app host.

use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::cost_router::{
    best_of_n_enabled, plan_candidates, run_best_of_n, BestOfNOutcome, BestOfNPlan, BudgetVerdict,
    CandidateSource, CandidateVerdict, Difficulty, Tier, Verdict,
};

/// One sampled candidate: its dispatch index, the run id used to create/reap its
/// worktree, and the isolated worktree it is generated and verified in.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Dispatch order (0-based) — the deterministic key the brain selects on.
    pub index: usize,
    /// The id used to create and later reap this candidate's worktree.
    pub run_id: String,
    /// The isolated worktree this candidate is generated and verified in.
    pub worktree: PathBuf,
}

// ── Effect seams (mocked in tests; real impls bind to the live primitives) ────

/// Create/reap an isolated worktree per candidate — the isolation seam. Each
/// candidate MUST get a distinct worktree off the same baseline so one
/// candidate's edits can never affect another's `verify` (and so the samples stay
/// diverse). Real impl: [`GoalWorktrees`].
#[async_trait]
pub trait CandidateWorktrees: Sync {
    /// Create a fresh isolated worktree for candidate `index`.
    async fn create(&self, index: usize) -> Result<Candidate, String>;
    /// Reap a candidate's worktree (best-effort; failures are non-fatal).
    async fn reap(&self, candidate: &Candidate);
}

/// Run ONE cheap-tier candidate attempt inside its worktree — the summon seam.
/// The real impl dispatches a cheap-tier [`crate::agents::subagent_handler::run_subagent_task`]
/// with the goal's task, pointed at `candidate.worktree`.
#[async_trait]
pub trait CandidateGenerator: Sync {
    /// Produce a candidate edit inside `candidate.worktree`. An `Err` marks the
    /// candidate a non-starter (it is scored `Fail`, never selected).
    async fn generate(&self, candidate: &Candidate) -> Result<(), String>;
}

/// Run `verify` in a candidate's worktree — the execution oracle. Real impl:
/// [`VerifyToolVerifier`]. `true` ⇔ the candidate passed (selection-eligible).
#[async_trait]
pub trait CandidateVerifier: Sync {
    /// Verify `candidate.worktree`; `true` when the project's checks pass there.
    async fn verify(&self, candidate: &Candidate) -> bool;
}

/// Adopt the winning candidate's work into the goal — the only mutation of the
/// goal's own tree. Kept behind a seam because the real splice into the live goal
/// state machine is the flagged follow-up wiring.
#[async_trait]
pub trait WinnerApplier: Sync {
    /// Apply the winner's changes to the goal. `Err` surfaces as
    /// [`SamplingOutcome::ApplyFailed`] (the winner's worktree is still reaped).
    async fn apply(&self, winner: &Candidate) -> Result<(), String>;
}

// ── The live source: isolate → generate → verify → verdict ────────────────────

/// A [`CandidateSource`] that, for each candidate, creates an isolated worktree,
/// runs a cheap-tier generation in it, and verifies it — feeding the pass/fail
/// verdict to the brain. Records every created candidate so the orchestrator can
/// find the winner and reap them all.
struct LiveCandidateSource<'a> {
    worktrees: &'a dyn CandidateWorktrees,
    generator: &'a dyn CandidateGenerator,
    verifier: &'a dyn CandidateVerifier,
    created: Mutex<Vec<Candidate>>,
}

impl<'a> LiveCandidateSource<'a> {
    fn new(
        worktrees: &'a dyn CandidateWorktrees,
        generator: &'a dyn CandidateGenerator,
        verifier: &'a dyn CandidateVerifier,
    ) -> Self {
        Self {
            worktrees,
            generator,
            verifier,
            created: Mutex::new(Vec::new()),
        }
    }

    /// Drain the candidates created so far (for winner lookup + reaping).
    fn take_created(&self) -> Vec<Candidate> {
        let mut guard = self.created.lock().expect("created mutex poisoned");
        std::mem::take(&mut *guard)
    }

    /// A `Fail` verdict for `index` — a candidate that never got off the ground
    /// (worktree/generation error). It is scored, never selected.
    fn failed(index: usize) -> CandidateVerdict {
        CandidateVerdict {
            index,
            verdict: Verdict::Fail,
            test_signature: None,
        }
    }
}

#[async_trait]
impl CandidateSource for LiveCandidateSource<'_> {
    async fn attempt(&self, index: usize) -> CandidateVerdict {
        // Isolate: a fresh worktree off the baseline. A create failure fails only
        // THIS candidate — the run continues with the others.
        let candidate = match self.worktrees.create(index).await {
            Ok(candidate) => candidate,
            Err(_) => return Self::failed(index),
        };
        // Record it for reaping / winner lookup. The guard is dropped before the
        // next await (std Mutex guards are not Send).
        self.created
            .lock()
            .expect("created mutex poisoned")
            .push(candidate.clone());

        // Generate a candidate edit on the cheap tier, inside the worktree.
        if self.generator.generate(&candidate).await.is_err() {
            return Self::failed(index);
        }

        // Select by EXECUTION: the whole `verify` suite is the single test, so the
        // signature is absent (the brain then uses the first-passer fallback).
        let passed = self.verifier.verify(&candidate).await;
        CandidateVerdict {
            index,
            verdict: if passed { Verdict::Pass } else { Verdict::Fail },
            test_signature: None,
        }
    }
}

// ── The orchestration entrypoint ──────────────────────────────────────────────

/// What one bounded task's best-of-N run needs from the caller: the cheap tier it
/// runs on, its difficulty, the live spend verdict, and the requested candidate
/// count (usually [`crate::cost_router::load_best_of_n`]).
pub struct SamplingRequest {
    /// The cheap tier candidates run on (never the frontier — the brain enforces).
    pub tier: Tier,
    /// The task's difficulty (drives the difficulty router).
    pub difficulty: Difficulty,
    /// The live spend verdict (drives the cost gate / spend cap).
    pub budget: BudgetVerdict,
    /// The requested candidate count before the cost gate clamps it.
    pub requested_n: u8,
}

/// The result of a best-of-N sampling run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SamplingOutcome {
    /// The feature flag is off — sampling did not run (the default).
    Disabled,
    /// The plan did not sample (easy task / frontier tier / spend cap → N=1) —
    /// the caller runs its normal single-edit path. Not a failure.
    Skipped,
    /// A candidate passed `verify` and its work was applied to the goal.
    Applied {
        /// The chosen candidate's dispatch index.
        index: usize,
        /// Candidates sampled in total.
        sampled: usize,
        /// Candidates that passed `verify`.
        passers: usize,
        /// CodeT agreement size of the chosen cluster.
        agree: usize,
    },
    /// A passer was selected but applying it to the goal failed.
    ApplyFailed {
        /// The chosen candidate's dispatch index.
        index: usize,
        /// Why the apply failed.
        error: String,
    },
    /// N candidates ran but none passed `verify` — the caller falls through to the
    /// normal fix-loop / verifier-driven tier escalation (#742).
    NonePassed {
        /// Candidates sampled in total.
        sampled: usize,
    },
}

/// Run best-of-N for one bounded task. Honors the feature flag (default OFF), then
/// plans via the brain (difficulty router + cost gate) and delegates to
/// [`sample`]. The heavy work runs over the seams, so the caller supplies the live
/// (or mock) worktree/generator/verifier/applier implementations.
pub async fn run_best_of_n_sampling(
    req: SamplingRequest,
    worktrees: &dyn CandidateWorktrees,
    generator: &dyn CandidateGenerator,
    verifier: &dyn CandidateVerifier,
    applier: &dyn WinnerApplier,
) -> SamplingOutcome {
    // Feature flag: nothing samples until it is explicitly enabled.
    if !best_of_n_enabled() {
        return SamplingOutcome::Disabled;
    }
    // The brain's difficulty router + cost gate → a concrete, budget-safe N.
    let plan = plan_candidates(req.difficulty, req.tier, req.requested_n, req.budget);
    sample(&plan, worktrees, generator, verifier, applier).await
}

/// The testable sampling core: fan out `plan.n` isolated candidates, CodeT-select
/// the passer, apply the winner (or fall through), and reap every worktree. Split
/// from the flag/plan wrapper so the whole orchestration is unit-testable with
/// mocks and explicit plans (no global config).
async fn sample(
    plan: &BestOfNPlan,
    worktrees: &dyn CandidateWorktrees,
    generator: &dyn CandidateGenerator,
    verifier: &dyn CandidateVerifier,
    applier: &dyn WinnerApplier,
) -> SamplingOutcome {
    // A non-sampling plan (easy / frontier / spend-capped → N=1) does nothing.
    if !plan.samples() {
        return SamplingOutcome::Skipped;
    }

    let source = LiveCandidateSource::new(worktrees, generator, verifier);
    // The brain: fan out N INDEPENDENT candidates, verify each, select the passer.
    let outcome = run_best_of_n(plan, &source).await;
    // Every candidate that got a worktree — for winner lookup and reaping.
    let created = source.take_created();

    let result = match outcome {
        // The wrapper already guaranteed plan.samples(); Skipped here is defensive.
        BestOfNOutcome::Skipped => SamplingOutcome::Skipped,
        BestOfNOutcome::NonePassed { sampled } => SamplingOutcome::NonePassed { sampled },
        BestOfNOutcome::Selected {
            index,
            agree,
            passers,
            sampled,
        } => match created.iter().find(|candidate| candidate.index == index) {
            Some(winner) => match applier.apply(winner).await {
                Ok(()) => SamplingOutcome::Applied {
                    index,
                    sampled,
                    passers,
                    agree,
                },
                Err(error) => SamplingOutcome::ApplyFailed { index, error },
            },
            None => SamplingOutcome::ApplyFailed {
                index,
                error: "winning candidate worktree not found".to_string(),
            },
        },
    };

    // Reap every candidate worktree — the winner's work is already applied; the
    // losers are discarded. Best-of-N worktrees are ephemeral scratch.
    for candidate in &created {
        worktrees.reap(candidate).await;
    }
    result
}

// ── Real seams (thin wrappers over the live primitives) ───────────────────────

/// The real worktree seam: an isolated detached worktree per candidate off the
/// goal's baseline, via [`super::goal_engine::create_goal_worktree`] /
/// [`super::goal_engine::reap_goal_worktree`].
pub struct GoalWorktrees {
    /// The goal's repository root.
    pub repo: PathBuf,
    /// The baseline commit each candidate worktree is checked out at.
    pub baseline: String,
    /// A run-id prefix (typically the goal's run id) to namespace the candidates.
    pub run_prefix: String,
}

#[async_trait]
impl CandidateWorktrees for GoalWorktrees {
    async fn create(&self, index: usize) -> Result<Candidate, String> {
        let run_id = format!("{}-bon-{index}", self.run_prefix);
        let worktree =
            super::goal_engine::create_goal_worktree(&self.repo, &self.baseline, &run_id).await?;
        Ok(Candidate {
            index,
            run_id,
            worktree,
        })
    }

    async fn reap(&self, candidate: &Candidate) {
        // allow_unpushed = true: best-of-N candidates are ephemeral scratch — the
        // winner's work is applied before reaping and the losers are discarded, so
        // the unpushed-work guard must not keep these worktrees around.
        let _ = super::goal_engine::reap_goal_worktree(&self.repo, &candidate.run_id, true).await;
    }
}

/// The real verify seam: the project's `verify` tool, run in the candidate's
/// worktree. A clean (non-error) result — PASS or a fresh-scaffold NO-CHECKS — is
/// a pass; only an `is_error` result fails the candidate.
pub struct VerifyToolVerifier;

#[async_trait]
impl CandidateVerifier for VerifyToolVerifier {
    async fn verify(&self, candidate: &Candidate) -> bool {
        use super::developer::verify::{VerifyParams, VerifyTool};
        let result = VerifyTool::new()
            .verify_with_cwd(
                VerifyParams {
                    command: None,
                    path: None,
                    timeout_secs: None,
                },
                Some(candidate.worktree.as_path()),
            )
            .await;
        result.is_error != Some(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost_router::budget::{budget_verdict, BudgetConfig};

    // ── helpers ──────────────────────────────────────────────────────────────

    fn budget_ok() -> BudgetVerdict {
        budget_verdict(0.0, 0.0, &BudgetConfig::default())
    }

    fn plan(n: u8) -> BestOfNPlan {
        BestOfNPlan {
            n,
            tier: Tier::LocalFree,
        }
    }

    // ── mock seams ───────────────────────────────────────────────────────────

    /// Worktrees mock: hands out a distinct fake worktree per index, records
    /// create/reap, and can be told to fail creation for specific indices.
    struct MockWorktrees {
        fail_create: Vec<usize>,
        created: Mutex<Vec<usize>>,
        reaped: Mutex<Vec<usize>>,
    }
    impl MockWorktrees {
        fn new() -> Self {
            Self {
                fail_create: Vec::new(),
                created: Mutex::new(Vec::new()),
                reaped: Mutex::new(Vec::new()),
            }
        }
        fn failing(fail_create: Vec<usize>) -> Self {
            Self {
                fail_create,
                created: Mutex::new(Vec::new()),
                reaped: Mutex::new(Vec::new()),
            }
        }
        fn created(&self) -> Vec<usize> {
            let mut v = self.created.lock().unwrap().clone();
            v.sort_unstable();
            v
        }
        fn reaped(&self) -> Vec<usize> {
            let mut v = self.reaped.lock().unwrap().clone();
            v.sort_unstable();
            v
        }
    }
    #[async_trait::async_trait]
    impl CandidateWorktrees for MockWorktrees {
        async fn create(&self, index: usize) -> Result<Candidate, String> {
            if self.fail_create.contains(&index) {
                return Err(format!("create failed for {index}"));
            }
            self.created.lock().unwrap().push(index);
            Ok(Candidate {
                index,
                run_id: format!("run-{index}"),
                worktree: PathBuf::from(format!("/wt/{index}")),
            })
        }
        async fn reap(&self, candidate: &Candidate) {
            self.reaped.lock().unwrap().push(candidate.index);
        }
    }

    /// Generator mock: records which candidates it was asked to generate; fails
    /// the indices in `fail`.
    struct MockGenerator {
        fail: Vec<usize>,
        generated: Mutex<Vec<usize>>,
    }
    impl MockGenerator {
        fn new() -> Self {
            Self {
                fail: Vec::new(),
                generated: Mutex::new(Vec::new()),
            }
        }
        fn failing(fail: Vec<usize>) -> Self {
            Self {
                fail,
                generated: Mutex::new(Vec::new()),
            }
        }
        fn generated(&self) -> Vec<usize> {
            let mut v = self.generated.lock().unwrap().clone();
            v.sort_unstable();
            v
        }
    }
    #[async_trait::async_trait]
    impl CandidateGenerator for MockGenerator {
        async fn generate(&self, candidate: &Candidate) -> Result<(), String> {
            self.generated.lock().unwrap().push(candidate.index);
            if self.fail.contains(&candidate.index) {
                Err("generate failed".to_string())
            } else {
                Ok(())
            }
        }
    }

    /// Verifier mock: passes the indices in `pass`; records each (index, worktree)
    /// it verified so isolation can be asserted.
    struct MockVerifier {
        pass: Vec<usize>,
        verified: Mutex<Vec<(usize, PathBuf)>>,
    }
    impl MockVerifier {
        fn passing(pass: Vec<usize>) -> Self {
            Self {
                pass,
                verified: Mutex::new(Vec::new()),
            }
        }
        fn verified_indices(&self) -> Vec<usize> {
            let mut v: Vec<usize> = self
                .verified
                .lock()
                .unwrap()
                .iter()
                .map(|(i, _)| *i)
                .collect();
            v.sort_unstable();
            v
        }
        fn verified_worktrees(&self) -> Vec<PathBuf> {
            self.verified
                .lock()
                .unwrap()
                .iter()
                .map(|(_, p)| p.clone())
                .collect()
        }
    }
    #[async_trait::async_trait]
    impl CandidateVerifier for MockVerifier {
        async fn verify(&self, candidate: &Candidate) -> bool {
            self.verified
                .lock()
                .unwrap()
                .push((candidate.index, candidate.worktree.clone()));
            self.pass.contains(&candidate.index)
        }
    }

    /// Applier mock: records the winners applied; can be told to fail.
    struct MockApplier {
        fail: bool,
        applied: Mutex<Vec<usize>>,
    }
    impl MockApplier {
        fn new() -> Self {
            Self {
                fail: false,
                applied: Mutex::new(Vec::new()),
            }
        }
        fn failing() -> Self {
            Self {
                fail: true,
                applied: Mutex::new(Vec::new()),
            }
        }
        fn applied(&self) -> Vec<usize> {
            self.applied.lock().unwrap().clone()
        }
    }
    #[async_trait::async_trait]
    impl WinnerApplier for MockApplier {
        async fn apply(&self, winner: &Candidate) -> Result<(), String> {
            self.applied.lock().unwrap().push(winner.index);
            if self.fail {
                Err("apply failed".to_string())
            } else {
                Ok(())
            }
        }
    }

    // ── the sampling core ────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_non_sampling_plan_is_skipped_and_touches_nothing() {
        let wt = MockWorktrees::new();
        let gen = MockGenerator::new();
        let ver = MockVerifier::passing(vec![]);
        let app = MockApplier::new();
        let out = sample(&plan(1), &wt, &gen, &ver, &app).await;
        assert_eq!(out, SamplingOutcome::Skipped);
        assert!(wt.created().is_empty(), "N=1 must not create worktrees");
        assert!(gen.generated().is_empty());
        assert!(ver.verified_indices().is_empty());
    }

    #[tokio::test]
    async fn fans_out_isolates_verifies_selects_applies_and_reaps() {
        // Four candidates; only #2 passes → it is applied. Every candidate is
        // created, generated, verified in ITS OWN worktree, and reaped.
        let wt = MockWorktrees::new();
        let gen = MockGenerator::new();
        let ver = MockVerifier::passing(vec![2]);
        let app = MockApplier::new();

        let out = sample(&plan(4), &wt, &gen, &ver, &app).await;
        assert_eq!(
            out,
            SamplingOutcome::Applied {
                index: 2,
                sampled: 4,
                passers: 1,
                agree: 1,
            }
        );
        // Fan-out: all four dispatched through every stage.
        assert_eq!(wt.created(), vec![0, 1, 2, 3]);
        assert_eq!(gen.generated(), vec![0, 1, 2, 3]);
        assert_eq!(ver.verified_indices(), vec![0, 1, 2, 3]);
        // Isolation: four DISTINCT worktrees.
        let mut worktrees = ver.verified_worktrees();
        worktrees.sort();
        worktrees.dedup();
        assert_eq!(
            worktrees.len(),
            4,
            "each candidate must get its own worktree"
        );
        // The winner was applied; every worktree reaped.
        assert_eq!(app.applied(), vec![2]);
        assert_eq!(wt.reaped(), vec![0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn none_passing_falls_through_and_does_not_apply() {
        let wt = MockWorktrees::new();
        let gen = MockGenerator::new();
        let ver = MockVerifier::passing(vec![]); // nobody passes
        let app = MockApplier::new();

        let out = sample(&plan(3), &wt, &gen, &ver, &app).await;
        assert_eq!(out, SamplingOutcome::NonePassed { sampled: 3 });
        assert!(app.applied().is_empty(), "no winner → no apply");
        assert_eq!(wt.reaped(), vec![0, 1, 2], "all worktrees still reaped");
    }

    #[tokio::test]
    async fn several_passers_apply_the_first_and_reap_all() {
        // #1 and #3 pass (no test signatures) → the lowest-index passer wins.
        let wt = MockWorktrees::new();
        let gen = MockGenerator::new();
        let ver = MockVerifier::passing(vec![1, 3]);
        let app = MockApplier::new();

        let out = sample(&plan(4), &wt, &gen, &ver, &app).await;
        assert_eq!(
            out,
            SamplingOutcome::Applied {
                index: 1,
                sampled: 4,
                passers: 2,
                agree: 1,
            }
        );
        assert_eq!(app.applied(), vec![1]);
        assert_eq!(wt.reaped(), vec![0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn a_generation_failure_fails_only_that_candidate() {
        // #0 fails to generate → it is never verified and cannot win; #1 passes.
        let wt = MockWorktrees::new();
        let gen = MockGenerator::failing(vec![0]);
        let ver = MockVerifier::passing(vec![1]);
        let app = MockApplier::new();

        let out = sample(&plan(2), &wt, &gen, &ver, &app).await;
        assert_eq!(
            out,
            SamplingOutcome::Applied {
                index: 1,
                sampled: 2,
                passers: 1,
                agree: 1,
            }
        );
        // #0 was generated (and failed) but never verified; both still reaped.
        assert_eq!(gen.generated(), vec![0, 1]);
        assert_eq!(ver.verified_indices(), vec![1]);
        assert_eq!(wt.reaped(), vec![0, 1]);
    }

    #[tokio::test]
    async fn a_worktree_create_failure_fails_only_that_candidate() {
        // #0's worktree can't be created → it fails without generate/verify and is
        // never reaped (nothing to reap); #1 passes and wins.
        let wt = MockWorktrees::failing(vec![0]);
        let gen = MockGenerator::new();
        let ver = MockVerifier::passing(vec![1]);
        let app = MockApplier::new();

        let out = sample(&plan(2), &wt, &gen, &ver, &app).await;
        assert_eq!(
            out,
            SamplingOutcome::Applied {
                index: 1,
                sampled: 2,
                passers: 1,
                agree: 1,
            }
        );
        assert_eq!(wt.created(), vec![1], "only #1's worktree was created");
        assert_eq!(gen.generated(), vec![1], "#0 never reached generation");
        assert_eq!(wt.reaped(), vec![1], "only the created worktree is reaped");
    }

    #[tokio::test]
    async fn an_apply_failure_is_reported_and_worktrees_still_reaped() {
        let wt = MockWorktrees::new();
        let gen = MockGenerator::new();
        let ver = MockVerifier::passing(vec![0]);
        let app = MockApplier::failing();

        let out = sample(&plan(2), &wt, &gen, &ver, &app).await;
        assert_eq!(
            out,
            SamplingOutcome::ApplyFailed {
                index: 0,
                error: "apply failed".to_string(),
            }
        );
        assert_eq!(app.applied(), vec![0], "apply was attempted");
        assert_eq!(
            wt.reaped(),
            vec![0, 1],
            "worktrees reaped despite apply fail"
        );
    }

    // ── the plan wrapper (composition over the tested brain) ──────────────────

    #[tokio::test]
    async fn sample_honors_an_easy_plan_as_skipped() {
        // A frontier/easy/spend-capped task plans to N=1; the adapter skips it.
        let easy = plan_candidates(Difficulty::Easy, Tier::LocalFree, 4, budget_ok());
        assert_eq!(easy.n, 1, "brain gate: easy → N=1");
        let wt = MockWorktrees::new();
        let gen = MockGenerator::new();
        let ver = MockVerifier::passing(vec![]);
        let app = MockApplier::new();
        assert_eq!(
            sample(&easy, &wt, &gen, &ver, &app).await,
            SamplingOutcome::Skipped
        );
        assert!(wt.created().is_empty());
    }

    #[tokio::test]
    async fn sample_runs_a_hard_cheap_plan() {
        // A hard cheap-tier task plans to N>1; the adapter samples it.
        let hard = plan_candidates(Difficulty::Hard, Tier::LocalFree, 3, budget_ok());
        assert!(hard.samples(), "brain gate: hard cheap → N>1");
        let wt = MockWorktrees::new();
        let gen = MockGenerator::new();
        let ver = MockVerifier::passing(vec![0]);
        let app = MockApplier::new();
        let out = sample(&hard, &wt, &gen, &ver, &app).await;
        assert!(matches!(out, SamplingOutcome::Applied { index: 0, .. }));
        assert_eq!(wt.created(), vec![0, 1, 2]);
    }
}
