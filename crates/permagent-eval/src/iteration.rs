//! Bounded control/treatment iteration loops for harness improvement.
//!
//! This module is deliberately independent of model clients.  A caller supplies
//! an [`ArmRunner`] (normally an adapter around [`crate::runner::run_task`]);
//! tests supply a closure that returns deterministic observations.  This keeps
//! unit tests cheap and makes the control loop responsible for the things that
//! must not be left to a model: fixed task order, paired arms, run/budget caps,
//! metric collection, and graduation gates.

use anyhow::Result;

use crate::cost::CostReading;
use crate::metrics::{aggregate, Aggregate, TaskResult};
use crate::oracle::OracleOutcome;
use crate::paired::{compare, Arm, PairedReport};
use crate::task::Task;

/// Evidence emitted by a coding-DAG run.
///
/// The evaluator does not infer trust from prose in a model transcript.  The
/// harness adapter must provide structured evidence.  The default is all false
/// (unknown evidence is not silently promoted to compliant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagEvidence {
    /// The graph was non-empty, acyclic, and accepted by the control plane.
    pub valid_graph: bool,
    /// The graph was within its declared node/turn bound.
    pub bounded: bool,
    /// A route/worker choice was recorded before execution.
    pub routing_recorded: bool,
    /// At least one deterministic verifier ran and passed.
    pub verification_passed: bool,
    /// Any required reviewer gate was satisfied.
    pub review_gate_satisfied: bool,
    /// Landing evidence was current at the point the run reported success.
    pub landing_evidence_current: bool,
    /// Number of observed mutation-boundary violations.
    pub mutation_boundary_violations: usize,
    /// Nodes declared by the run's DAG.
    pub node_count: usize,
    /// Nodes completed by the run.
    pub completed_nodes: usize,
    /// Verification attempts, including failed attempts.
    pub verification_attempts: usize,
}

impl DagEvidence {
    /// Evidence suitable for a synthetic test that models a fully compliant
    /// run.  Production adapters should populate the fields from receipts.
    pub fn excellent(node_count: usize) -> Self {
        Self {
            valid_graph: node_count > 0,
            bounded: true,
            routing_recorded: true,
            verification_passed: true,
            review_gate_satisfied: true,
            landing_evidence_current: true,
            mutation_boundary_violations: 0,
            node_count,
            completed_nodes: node_count,
            verification_attempts: 1,
        }
    }

    /// No trustworthy DAG receipt was available.
    pub fn unknown() -> Self {
        Self {
            valid_graph: false,
            bounded: false,
            routing_recorded: false,
            verification_passed: false,
            review_gate_satisfied: false,
            landing_evidence_current: false,
            mutation_boundary_violations: 0,
            node_count: 0,
            completed_nodes: 0,
            verification_attempts: 0,
        }
    }

    /// Whether all trust conditions needed for graduation hold.
    pub fn compliant(&self) -> bool {
        self.valid_graph
            && self.bounded
            && self.routing_recorded
            && self.verification_passed
            && self.review_gate_satisfied
            && self.landing_evidence_current
            && self.mutation_boundary_violations == 0
            && self.node_count > 0
            && self.completed_nodes == self.node_count
            && self.verification_attempts > 0
    }
}

/// Output of one arm invocation, kept separate from arm/repetition metadata
/// owned by the loop.
#[derive(Debug, Clone)]
pub struct ArmRun {
    pub result: TaskResult,
    pub dag: DagEvidence,
}

/// One observed task/arm/repetition cell.
#[derive(Debug, Clone)]
pub struct Observation {
    pub task_id: String,
    pub arm: Arm,
    pub repetition: usize,
    pub result: TaskResult,
    pub dag: DagEvidence,
}

/// The only model-dependent seam.  Production code can adapt the existing
/// runner traits; unit tests can implement this with a pure closure.
pub trait ArmRunner {
    fn run(&mut self, task: &Task, arm: Arm, repetition: usize) -> Result<ArmRun>;
}

impl<F> ArmRunner for F
where
    F: FnMut(&Task, Arm, usize) -> Result<ArmRun>,
{
    fn run(&mut self, task: &Task, arm: Arm, repetition: usize) -> Result<ArmRun> {
        self(task, arm, repetition)
    }
}

/// Limits and gates for one bounded training loop.
#[derive(Debug, Clone)]
pub struct IterationConfig {
    /// Number of control/treatment passes over the fixed task set per round.
    pub repetitions_per_iteration: usize,
    /// Maximum number of model runs across both arms and all iterations.
    pub max_total_runs: usize,
    /// Shared measured-spend cap across both arms and all iterations.
    pub budget_usd: Option<f64>,
    /// Maximum number of rounds before the loop stops with `Hold`.
    pub max_iterations: usize,
    pub gates: GraduationGates,
}

impl Default for IterationConfig {
    fn default() -> Self {
        Self {
            repetitions_per_iteration: 1,
            max_total_runs: 512,
            budget_usd: None,
            max_iterations: 1,
            gates: GraduationGates::default(),
        }
    }
}

impl IterationConfig {
    pub fn validate(&self) -> Result<()> {
        if self.repetitions_per_iteration == 0 {
            anyhow::bail!("repetitions_per_iteration must be greater than zero");
        }
        if self.max_total_runs == 0 {
            anyhow::bail!("max_total_runs must be greater than zero");
        }
        if self.max_iterations == 0 {
            anyhow::bail!("max_iterations must be greater than zero");
        }
        if let Some(cap) = self.budget_usd {
            if !cap.is_finite() || cap < 0.0 {
                anyhow::bail!("budget_usd must be finite and non-negative");
            }
        }
        self.gates.validate()
    }
}

/// Graduation criteria.  A `None` threshold means that metric is recorded but
/// not used as a gate.  The default requires a meaningful paired sample, no
/// treatment regression, and complete structured DAG evidence.
#[derive(Debug, Clone)]
pub struct GraduationGates {
    pub min_runs_per_arm: usize,
    /// Number of consecutive snapshots satisfying every gate before
    /// promotion. This prevents a one-round fluke from graduating.
    pub consecutive_qualifying_iterations: usize,
    /// Absolute quality bar for the treatment arm. The control arm is only a
    /// paired baseline and is intentionally not held to this bar.
    pub min_pass_rate: f64,
    pub min_treatment_delta: f64,
    pub max_median_duration_secs: Option<f64>,
    pub max_dollars_per_solved: Option<f64>,
    /// Optional relative reduction required in treatment versus control.
    /// Values are fractions (0.10 means at least 10% lower).
    pub min_token_reduction_fraction: Option<f64>,
    pub min_cost_reduction_fraction: Option<f64>,
    pub require_dag_compliance: bool,
    pub require_known_costs: bool,
    pub require_token_metrics: bool,
}

impl Default for GraduationGates {
    fn default() -> Self {
        Self {
            min_runs_per_arm: crate::paired::MIN_RUNS_PER_ARM_FOR_CONFIDENCE,
            consecutive_qualifying_iterations: 3,
            min_pass_rate: 0.80,
            min_treatment_delta: 0.0,
            max_median_duration_secs: None,
            max_dollars_per_solved: None,
            min_token_reduction_fraction: None,
            min_cost_reduction_fraction: None,
            require_dag_compliance: true,
            require_known_costs: false,
            require_token_metrics: false,
        }
    }
}

impl GraduationGates {
    fn validate(&self) -> Result<()> {
        if self.consecutive_qualifying_iterations == 0 {
            anyhow::bail!("consecutive_qualifying_iterations must be greater than zero");
        }
        if !(0.0..=1.0).contains(&self.min_pass_rate) {
            anyhow::bail!("min_pass_rate must be in [0, 1]");
        }
        if let Some(v) = self.max_median_duration_secs {
            if !v.is_finite() || v < 0.0 {
                anyhow::bail!("max_median_duration_secs must be finite and non-negative");
            }
        }
        if let Some(v) = self.max_dollars_per_solved {
            if !v.is_finite() || v < 0.0 {
                anyhow::bail!("max_dollars_per_solved must be finite and non-negative");
            }
        }
        for (name, value) in [
            (
                "min_token_reduction_fraction",
                self.min_token_reduction_fraction,
            ),
            (
                "min_cost_reduction_fraction",
                self.min_cost_reduction_fraction,
            ),
        ] {
            if let Some(v) = value {
                if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                    anyhow::bail!("{name} must be finite and in [0, 1]");
                }
            }
        }
        Ok(())
    }
}

/// Token totals retained separately because `Aggregate` intentionally keeps
/// its historical surface compact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenTotals {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
}

fn sum_tokens(results: &[TaskResult], field: impl Fn(&CostReading) -> Option<i64>) -> Option<i64> {
    let mut sum = 0_i64;
    let mut known = false;
    for result in results.iter().filter(|r| r.oracle != OracleOutcome::NotRun) {
        if let Some(value) = field(&result.cost) {
            sum = sum.saturating_add(value);
            known = true;
        }
    }
    known.then_some(sum)
}

fn token_totals(results: &[TaskResult]) -> TokenTotals {
    TokenTotals {
        input_tokens: sum_tokens(results, |c| c.input_tokens),
        output_tokens: sum_tokens(results, |c| c.output_tokens),
        cache_read_tokens: sum_tokens(results, |c| c.cache_read_tokens),
        cache_write_tokens: sum_tokens(results, |c| c.cache_write_tokens),
    }
}

/// Aggregate DAG evidence for one arm.
#[derive(Debug, Clone, PartialEq)]
pub struct DagSummary {
    pub runs: usize,
    pub compliant: usize,
    pub compliance_rate: f64,
    pub total_nodes: usize,
    pub completed_nodes: usize,
    pub verification_attempts: usize,
    pub mutation_boundary_violations: usize,
}

fn dag_summary(observations: &[Observation]) -> DagSummary {
    let attempted: Vec<&Observation> = observations
        .iter()
        .filter(|o| o.result.oracle != OracleOutcome::NotRun)
        .collect();
    let compliant = attempted.iter().filter(|o| o.dag.compliant()).count();
    DagSummary {
        runs: attempted.len(),
        compliant,
        compliance_rate: if attempted.is_empty() {
            0.0
        } else {
            compliant as f64 / attempted.len() as f64
        },
        total_nodes: attempted.iter().map(|o| o.dag.node_count).sum(),
        completed_nodes: attempted.iter().map(|o| o.dag.completed_nodes).sum(),
        verification_attempts: attempted.iter().map(|o| o.dag.verification_attempts).sum(),
        mutation_boundary_violations: attempted
            .iter()
            .map(|o| o.dag.mutation_boundary_violations)
            .sum(),
    }
}

/// Metrics for one arm over all observations collected so far.
#[derive(Debug, Clone)]
pub struct ArmMetrics {
    pub aggregate: Aggregate,
    pub tokens: TokenTotals,
    pub dag: DagSummary,
}

/// Why a proposed treatment is not yet allowed to graduate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraduationStatus {
    Graduated,
    Hold,
    Rejected,
}

/// Gate result. Reasons are intentionally plain text so CLI/report layers can
/// show exactly which bar failed without reproducing gate logic.
#[derive(Debug, Clone, PartialEq)]
pub struct GraduationDecision {
    pub status: GraduationStatus,
    pub reasons: Vec<String>,
}

/// A complete paired snapshot after one iteration.
#[derive(Debug, Clone)]
pub struct IterationReport {
    pub iteration: usize,
    pub control: ArmMetrics,
    pub treatment: ArmMetrics,
    pub paired: PairedReport,
    pub decision: GraduationDecision,
    pub budget_spent_usd: f64,
    pub budget_cap_usd: Option<f64>,
    pub launched_runs: usize,
    pub skipped_runs: usize,
    pub observations: Vec<Observation>,
}

/// All snapshots from a bounded training loop.
#[derive(Debug, Clone)]
pub struct TrainingReport {
    pub iterations: Vec<IterationReport>,
    pub final_decision: GraduationDecision,
    pub budget_spent_usd: f64,
    pub budget_cap_usd: Option<f64>,
    pub launched_runs: usize,
    pub skipped_runs: usize,
}

#[derive(Debug, Clone, Copy)]
struct Budget {
    cap: Option<f64>,
    spent: f64,
    launched: usize,
    stopped: bool,
}

impl Budget {
    fn new(cap: Option<f64>) -> Self {
        Self {
            cap,
            spent: 0.0,
            launched: 0,
            stopped: false,
        }
    }

    fn can_launch(&self, max_total_runs: usize) -> bool {
        !self.stopped && self.launched < max_total_runs
    }

    fn record(&mut self, cost: Option<f64>) {
        self.launched += 1;
        self.spent += cost.unwrap_or(0.0);
        if let Some(cap) = self.cap {
            // The run that crosses the cap is retained; subsequent cells are
            // explicitly skipped.  This mirrors BudgetTracker's semantics.
            self.stopped = self.spent > cap;
        }
    }
}

fn not_run(task: &Task, arm: Arm, repetition: usize, reason: &str) -> Observation {
    let mut result = TaskResult::new(
        task.spec.id.clone(),
        task.spec.category.clone(),
        OracleOutcome::NotRun,
        CostReading::unknown(),
    );
    result.note = Some(reason.to_string());
    Observation {
        task_id: task.spec.id.clone(),
        arm,
        repetition,
        result,
        dag: DagEvidence::unknown(),
    }
}

fn errored(task: &Task, arm: Arm, repetition: usize, message: String) -> Observation {
    let mut result = TaskResult::new(
        task.spec.id.clone(),
        task.spec.category.clone(),
        OracleOutcome::Errored,
        CostReading::unknown(),
    );
    result.note = Some(message);
    Observation {
        task_id: task.spec.id.clone(),
        arm,
        repetition,
        result,
        dag: DagEvidence::unknown(),
    }
}

fn metrics_for(observations: &[Observation], arm: Arm) -> ArmMetrics {
    let results: Vec<TaskResult> = observations
        .iter()
        .filter(|o| o.arm == arm)
        .map(|o| o.result.clone())
        .collect();
    ArmMetrics {
        aggregate: aggregate(&results),
        tokens: token_totals(&results),
        dag: dag_summary(
            &observations
                .iter()
                .filter(|o| o.arm == arm)
                .cloned()
                .collect::<Vec<_>>(),
        ),
    }
}

fn decision(
    control: &ArmMetrics,
    treatment: &ArmMetrics,
    paired: &PairedReport,
    gates: &GraduationGates,
) -> GraduationDecision {
    let mut hold = Vec::new();
    let mut reject = Vec::new();
    // Both arms need enough observations to make a paired comparison and must
    // provide the evidence required by the selected experiment. Only the
    // treatment is held to the absolute pass-rate bar: a treatment is allowed
    // to improve a weak baseline.
    for (label, metrics) in [("control", control), ("treatment", treatment)] {
        if metrics.aggregate.attempted < gates.min_runs_per_arm {
            hold.push(format!(
                "{label} has {} attempted run(s); need at least {}",
                metrics.aggregate.attempted, gates.min_runs_per_arm
            ));
        }
        if label == "treatment" && metrics.aggregate.pass_rate < gates.min_pass_rate {
            hold.push(format!(
                "{label} pass-rate {:.1}% is below {:.1}%",
                metrics.aggregate.pass_rate * 100.0,
                gates.min_pass_rate * 100.0
            ));
        }
        if gates.require_dag_compliance && metrics.dag.compliance_rate < 1.0 {
            hold.push(format!(
                "{label} DAG compliance is {:.1}% (all attempted runs must be compliant)",
                metrics.dag.compliance_rate * 100.0
            ));
        }
        if gates.require_known_costs && metrics.aggregate.any_cost_unknown {
            hold.push(format!("{label} has unknown cost readings"));
        }
        if gates.require_token_metrics
            && (metrics.tokens.input_tokens.is_none() || metrics.tokens.output_tokens.is_none())
        {
            hold.push(format!("{label} is missing input/output token readings"));
        }
    }
    // Do not reject a treatment on an under-sized sample.  Until both arms
    // clear the confidence floor, the delta is diagnostic noise and the only
    // honest state is `Hold`.
    if control.aggregate.attempted >= gates.min_runs_per_arm
        && treatment.aggregate.attempted >= gates.min_runs_per_arm
        && paired.delta < gates.min_treatment_delta
    {
        reject.push(format!(
            "treatment delta {:+.1} points is below the required {:+.1} points",
            paired.delta * 100.0,
            gates.min_treatment_delta * 100.0
        ));
    }
    if let Some(limit) = gates.max_median_duration_secs {
        match treatment.aggregate.median_duration_secs {
            Some(value) if value <= limit => {}
            Some(value) => hold.push(format!(
                "treatment median latency {:.2}s exceeds {:.2}s",
                value, limit
            )),
            None => hold.push("treatment median latency is unavailable".to_string()),
        }
    }
    if let Some(limit) = gates.max_dollars_per_solved {
        match treatment.aggregate.dollars_per_solved {
            Some(value) if value <= limit => {}
            Some(value) => hold.push(format!(
                "treatment $/solved ${value:.4} exceeds ${limit:.4}"
            )),
            None => hold.push("treatment $/solved is unavailable".to_string()),
        }
    }
    if let Some(required) = gates.min_token_reduction_fraction {
        let control_tokens = total_tokens(control.tokens);
        let treatment_tokens = total_tokens(treatment.tokens);
        match (control_tokens, treatment_tokens) {
            (Some(c), Some(t)) if (t as f64) <= (c as f64) * (1.0 - required) => {}
            (Some(c), Some(t)) => hold.push(format!(
                "treatment tokens {t} do not reduce control {c} by at least {:.1}%",
                required * 100.0
            )),
            _ => hold.push("relative token reduction is unavailable".to_string()),
        }
    }
    if let Some(required) = gates.min_cost_reduction_fraction {
        match (
            control.aggregate.total_cost_usd,
            treatment.aggregate.total_cost_usd,
        ) {
            (Some(c), Some(t)) if t <= c * (1.0 - required) => {}
            (Some(c), Some(t)) => hold.push(format!(
                "treatment cost ${t:.4} does not reduce control ${c:.4} by at least {:.1}%",
                required * 100.0
            )),
            _ => hold.push("relative cost reduction is unavailable".to_string()),
        }
    }
    let status = if !reject.is_empty() {
        GraduationStatus::Rejected
    } else if !hold.is_empty() {
        GraduationStatus::Hold
    } else {
        GraduationStatus::Graduated
    };
    let reasons = reject.into_iter().chain(hold).collect();
    GraduationDecision { status, reasons }
}

fn total_tokens(tokens: TokenTotals) -> Option<i64> {
    Some(tokens.input_tokens?.saturating_add(tokens.output_tokens?))
}

fn snapshot(
    iteration: usize,
    observations: Vec<Observation>,
    budget: &Budget,
    gates: &GraduationGates,
) -> IterationReport {
    let control = metrics_for(&observations, Arm::Control);
    let treatment = metrics_for(&observations, Arm::Treatment);
    let control_results: Vec<TaskResult> = observations
        .iter()
        .filter(|o| o.arm == Arm::Control)
        .map(|o| o.result.clone())
        .filter(|r| r.oracle != OracleOutcome::NotRun)
        .collect();
    let treatment_results: Vec<TaskResult> = observations
        .iter()
        .filter(|o| o.arm == Arm::Treatment)
        .map(|o| o.result.clone())
        .filter(|r| r.oracle != OracleOutcome::NotRun)
        .collect();
    let paired = compare(&control_results, &treatment_results);
    let decision = decision(&control, &treatment, &paired, gates);
    let skipped_runs = observations
        .iter()
        .filter(|o| o.result.oracle == OracleOutcome::NotRun)
        .count();
    IterationReport {
        iteration,
        control,
        treatment,
        paired,
        decision,
        budget_spent_usd: budget.spent,
        budget_cap_usd: budget.cap,
        launched_runs: budget.launched,
        skipped_runs,
        observations,
    }
}

/// Run a bounded control/treatment training loop over the exact task order
/// supplied by the caller.  Each task is paired control-first, then treatment,
/// for each repetition.  A budget or run cap marks all remaining cells
/// `NotRun`; they are never counted as failures.
pub fn run_training_loop<R: ArmRunner>(
    tasks: &[Task],
    config: &IterationConfig,
    runner: &mut R,
) -> Result<TrainingReport> {
    config.validate()?;
    if tasks.is_empty() {
        anyhow::bail!("training loop requires at least one fixed task");
    }
    let mut budget = Budget::new(config.budget_usd);
    let mut all = Vec::new();
    let mut reports = Vec::new();
    let mut qualifying_streak = 0_usize;

    for iteration in 0..config.max_iterations {
        let mut round = Vec::new();
        for repetition in 0..config.repetitions_per_iteration {
            for task in tasks {
                for arm in [Arm::Control, Arm::Treatment] {
                    if !budget.can_launch(config.max_total_runs) {
                        round.push(not_run(
                            task,
                            arm,
                            repetition,
                            if budget.stopped {
                                "skipped: shared budget cap reached"
                            } else {
                                "skipped: max_total_runs reached"
                            },
                        ));
                        continue;
                    }
                    let observation = match runner.run(task, arm, repetition) {
                        Ok(run) => {
                            // The fixed task slice owns identity; an adapter
                            // cannot accidentally attribute a result to a
                            // different task by returning stale metadata.
                            let mut result = run.result;
                            result.task_id = task.spec.id.clone();
                            result.category = task.spec.category.clone();
                            Observation {
                                task_id: task.spec.id.clone(),
                                arm,
                                repetition,
                                result,
                                dag: run.dag,
                            }
                        }
                        Err(error) => errored(task, arm, repetition, format!("run error: {error}")),
                    };
                    budget.record(observation.result.cost.usd);
                    round.push(observation);
                }
            }
        }
        all.extend(round);
        let mut report = snapshot(iteration, all.clone(), &budget, &config.gates);
        if report.decision.status == GraduationStatus::Graduated {
            qualifying_streak += 1;
            if qualifying_streak < config.gates.consecutive_qualifying_iterations {
                report.decision.status = GraduationStatus::Hold;
                report.decision.reasons.push(format!(
                    "{} consecutive qualifying iteration(s) so far; need {}",
                    qualifying_streak, config.gates.consecutive_qualifying_iterations
                ));
            }
        } else {
            qualifying_streak = 0;
        }
        let done = matches!(
            report.decision.status,
            GraduationStatus::Graduated | GraduationStatus::Rejected
        );
        reports.push(report);
        if done || budget.stopped || budget.launched >= config.max_total_runs {
            break;
        }
    }

    let final_decision = reports
        .last()
        .map(|r| r.decision.clone())
        .unwrap_or_else(|| GraduationDecision {
            status: GraduationStatus::Hold,
            reasons: vec!["no iteration was launched".to_string()],
        });
    Ok(TrainingReport {
        iterations: reports,
        final_decision,
        budget_spent_usd: budget.spent,
        budget_cap_usd: budget.cap,
        launched_runs: budget.launched,
        skipped_runs: all
            .iter()
            .filter(|o| o.result.oracle == OracleOutcome::NotRun)
            .count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::TaskSpec;
    use std::path::PathBuf;

    fn task(id: &str) -> Task {
        Task {
            spec: TaskSpec::from_yaml(&format!(
                "id: {id}\ntitle: Test\ncategory: test\nprompt: do it\ntest: [true]\n"
            ))
            .unwrap(),
            dir: PathBuf::from("/tmp").join(id),
        }
    }

    fn run_for(_arm: Arm, pass: bool, cost: f64) -> ArmRun {
        let outcome = if pass {
            OracleOutcome::Pass
        } else {
            OracleOutcome::Fail
        };
        ArmRun {
            result: TaskResult::new(
                "ignored-by-loop",
                "test",
                outcome,
                CostReading::known_with_tokens(
                    cost,
                    false,
                    1,
                    Some(100),
                    Some(20),
                    Some(25),
                    Some(5),
                ),
            ),
            dag: DagEvidence::excellent(3),
        }
    }

    fn gates(min_runs: usize) -> GraduationGates {
        GraduationGates {
            min_runs_per_arm: min_runs,
            consecutive_qualifying_iterations: 3,
            min_pass_rate: 0.5,
            min_treatment_delta: 0.0,
            max_median_duration_secs: None,
            max_dollars_per_solved: None,
            min_token_reduction_fraction: None,
            min_cost_reduction_fraction: None,
            require_dag_compliance: true,
            require_known_costs: true,
            require_token_metrics: true,
        }
    }

    #[test]
    fn deterministic_runner_records_both_arms_and_metrics_without_model_calls() {
        let tasks = vec![task("a"), task("b")];
        let mut calls = Vec::new();
        let mut runner = |t: &Task, arm: Arm, repetition: usize| {
            calls.push((t.spec.id.clone(), arm, repetition));
            Ok(run_for(arm, matches!(arm, Arm::Treatment), 0.25))
        };
        let config = IterationConfig {
            repetitions_per_iteration: 2,
            max_total_runs: 8,
            budget_usd: None,
            max_iterations: 1,
            gates: gates(4),
        };
        let report = run_training_loop(&tasks, &config, &mut runner).unwrap();
        assert_eq!(calls.len(), 8);
        assert_eq!(calls[0], ("a".into(), Arm::Control, 0));
        assert_eq!(calls[1], ("a".into(), Arm::Treatment, 0));
        assert_eq!(report.launched_runs, 8);
        let iteration = &report.iterations[0];
        assert_eq!(iteration.control.aggregate.solved, 0);
        assert_eq!(iteration.treatment.aggregate.solved, 4);
        assert!(iteration
            .observations
            .iter()
            .all(|o| o.task_id == o.result.task_id && o.result.category == "test"));
        assert_eq!(iteration.treatment.tokens.input_tokens, Some(400));
        assert_eq!(iteration.treatment.dag.compliant, 4);
        assert_eq!(report.final_decision.status, GraduationStatus::Hold);
    }

    #[test]
    fn budget_marks_remaining_cells_not_run_and_never_as_failures() {
        let tasks = vec![task("a"), task("b")];
        let mut runner = |_t: &Task, _arm: Arm, _rep: usize| Ok(run_for(Arm::Control, true, 0.6));
        let config = IterationConfig {
            repetitions_per_iteration: 2,
            max_total_runs: 99,
            budget_usd: Some(1.0),
            max_iterations: 4,
            gates: gates(1),
        };
        let report = run_training_loop(&tasks, &config, &mut runner).unwrap();
        assert_eq!(report.launched_runs, 2);
        assert!(report.skipped_runs > 0);
        assert_eq!(report.iterations[0].control.aggregate.attempted, 1);
        assert_eq!(report.iterations[0].treatment.aggregate.attempted, 1);
        assert_eq!(report.iterations[0].control.aggregate.not_run, 3);
        assert!(report
            .iterations
            .last()
            .unwrap()
            .observations
            .iter()
            .any(|o| o.result.oracle == OracleOutcome::NotRun));
    }

    #[test]
    fn unknown_dag_evidence_cannot_graduate_even_with_passing_tasks() {
        let tasks = vec![task("a")];
        let mut runner = |_t: &Task, arm: Arm, _rep: usize| {
            let mut run = run_for(arm, true, 0.0);
            run.dag = DagEvidence::unknown();
            Ok(run)
        };
        let config = IterationConfig {
            repetitions_per_iteration: 1,
            max_total_runs: 2,
            budget_usd: None,
            max_iterations: 1,
            gates: gates(1),
        };
        let report = run_training_loop(&tasks, &config, &mut runner).unwrap();
        assert_eq!(report.final_decision.status, GraduationStatus::Hold);
        assert!(report
            .final_decision
            .reasons
            .iter()
            .any(|r| r.contains("DAG")));
    }

    #[test]
    fn a_negative_delta_below_sample_floor_is_hold_not_reject() {
        let tasks = vec![task("a")];
        let mut runner =
            |_t: &Task, arm: Arm, _rep: usize| Ok(run_for(arm, matches!(arm, Arm::Control), 0.0));
        let config = IterationConfig {
            repetitions_per_iteration: 1,
            max_total_runs: 2,
            budget_usd: None,
            max_iterations: 1,
            gates: gates(30),
        };
        let report = run_training_loop(&tasks, &config, &mut runner).unwrap();
        assert_eq!(report.final_decision.status, GraduationStatus::Hold);
        assert!(!report
            .final_decision
            .reasons
            .iter()
            .any(|r| r.contains("delta")));
    }

    #[test]
    fn validation_rejects_unbounded_or_invalid_configuration() {
        let mut config = IterationConfig {
            max_total_runs: 0,
            ..IterationConfig::default()
        };
        assert!(config.validate().is_err());
        config.max_total_runs = 1;
        config.budget_usd = Some(-1.0);
        assert!(config.validate().is_err());
        config.budget_usd = None;
        config.gates.min_pass_rate = 2.0;
        assert!(config.validate().is_err());
        config.gates.min_pass_rate = 0.5;
        config.gates.consecutive_qualifying_iterations = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn treatment_can_improve_a_weak_control_but_needs_three_rounds() {
        let tasks = vec![task("a")];
        let mut runner = |_t: &Task, arm: Arm, _rep: usize| {
            Ok(run_for(arm, matches!(arm, Arm::Treatment), 0.25))
        };
        let config = IterationConfig {
            repetitions_per_iteration: 1,
            max_total_runs: 6,
            budget_usd: None,
            max_iterations: 5,
            gates: gates(1),
        };
        let report = run_training_loop(&tasks, &config, &mut runner).unwrap();
        assert_eq!(report.iterations.len(), 3);
        assert_eq!(report.final_decision.status, GraduationStatus::Graduated);
        assert!(report.iterations[0]
            .decision
            .reasons
            .iter()
            .any(|r| r.contains("consecutive")));
        assert_eq!(report.iterations[2].treatment.aggregate.pass_rate, 1.0);
        assert_eq!(report.iterations[2].control.aggregate.pass_rate, 0.0);
    }
}
