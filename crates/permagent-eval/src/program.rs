//! Deterministic validation for a program DAG whose nodes are child DAGs.
//!
//! This is the continuity contract above individual coding roadmaps. It does
//! not schedule work or create another goal store: Permagent's existing goal
//! roadmap remains the runtime scheduler and Spectral/session storage remains
//! the durable source of truth. The program manifest only proves that an
//! incomplete improvement program always has an active, ready, or explicitly
//! blocked frontier and that every child DAG leads to the declared terminal
//! gate.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramNodeStatus {
    Passed,
    Active,
    Planned,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    None,
    Human,
    SpendCap,
}

fn default_approval() -> ApprovalPolicy {
    ApprovalPolicy::None
}

/// How a completed child DAG delivers its result. Code work must be landed on
/// the project trunk before a successor can run; explicitly approved research
/// or audit work has no repository delta and therefore has no landing step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Code,
    NoWrite,
}

fn default_delivery() -> DeliveryMode {
    DeliveryMode::Code
}

fn is_default_delivery(delivery: &DeliveryMode) -> bool {
    *delivery == DeliveryMode::Code
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramNode {
    pub id: String,
    pub child_dag: String,
    pub status: ProgramNodeStatus,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub next_on_pass: Vec<String>,
    pub entry_gate: Vec<String>,
    pub exit_gate: Vec<String>,
    pub worker_policy: String,
    #[serde(default = "default_approval")]
    pub approval: ApprovalPolicy,
    #[serde(
        default = "default_delivery",
        skip_serializing_if = "is_default_delivery"
    )]
    pub delivery: DeliveryMode,
    #[serde(default)]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramDag {
    pub schema: u32,
    pub program_id: String,
    pub objective: String,
    pub terminal_node: String,
    pub nodes: Vec<ProgramNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramFrontier {
    pub active: Vec<String>,
    pub ready: Vec<String>,
    /// Dependency-ready nodes which are waiting for a non-automatic approval.
    /// These remain `Planned`; this is a read-only projection of the frontier,
    /// not a second scheduler state.
    pub approval_required: Vec<String>,
    pub blocked: Vec<String>,
    pub complete: bool,
}

/// A receipt for one declared exit gate of a child DAG.
///
/// The controller deliberately accepts receipts as data rather than trying to
/// infer completion from child logs or prompt text.  A transition requires an
/// exact, duplicate-free set of receipts for every exit gate on the node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitGateReceipt {
    pub gate: String,
    pub passed: bool,
    /// Identity of the trusted verification run that produced this receipt.
    /// Automatic continuation must never consume a gate set from an earlier
    /// verdict; older explicit CLI receipts remain wire-compatible because
    /// this field is optional on deserialization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_id: Option<String>,
}

impl ExitGateReceipt {
    pub fn passed(gate: impl Into<String>) -> Self {
        Self {
            gate: gate.into(),
            passed: true,
            verification_id: None,
        }
    }

    pub fn failed(gate: impl Into<String>) -> Self {
        Self {
            gate: gate.into(),
            passed: false,
            verification_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramTransition {
    pub node_id: String,
    pub activated: Vec<String>,
    pub approval_required: Vec<String>,
    pub frontier: ProgramFrontier,
}

/// Result of reopening an earlier child DAG after a retained regression.
///
/// The caller remains responsible for persisting `reason` through the existing
/// run/session evidence path. Keeping the reason in the receipt, rather than in
/// a second program store, lets the controller enforce an explainable reopen
/// without becoming another memory system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramReopen {
    pub node_id: String,
    pub reason: String,
    pub reset_descendants: Vec<String>,
    pub approval_required: bool,
    pub frontier: ProgramFrontier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramTransitionError {
    UnknownNode(String),
    NodeNotActive {
        node_id: String,
        status: ProgramNodeStatus,
    },
    MissingExitGate {
        node_id: String,
        gate: String,
    },
    UnexpectedExitGate {
        node_id: String,
        gate: String,
    },
    DuplicateExitReceipt {
        node_id: String,
        gate: String,
    },
    ExitGateFailed {
        node_id: String,
        gate: String,
    },
    InvalidProgram(String),
    EmptyFrontier,
}

impl fmt::Display for ProgramTransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNode(node_id) => write!(f, "unknown program node '{node_id}'"),
            Self::NodeNotActive { node_id, status } => {
                write!(
                    f,
                    "program node '{node_id}' is not active (status {status:?})"
                )
            }
            Self::MissingExitGate { node_id, gate } => {
                write!(f, "node '{node_id}' is missing exit-gate receipt '{gate}'")
            }
            Self::UnexpectedExitGate { node_id, gate } => {
                write!(
                    f,
                    "node '{node_id}' received unexpected exit-gate receipt '{gate}'"
                )
            }
            Self::DuplicateExitReceipt { node_id, gate } => {
                write!(
                    f,
                    "node '{node_id}' received duplicate exit-gate receipt '{gate}'"
                )
            }
            Self::ExitGateFailed { node_id, gate } => {
                write!(
                    f,
                    "node '{node_id}' has a failed exit-gate receipt '{gate}'"
                )
            }
            Self::InvalidProgram(reason) => {
                write!(f, "transition would leave invalid program: {reason}")
            }
            Self::EmptyFrontier => write!(
                f,
                "transition would leave an incomplete program with no frontier"
            ),
        }
    }
}

impl std::error::Error for ProgramTransitionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramReopenError {
    UnknownNode(String),
    NodeNotPassed {
        node_id: String,
        status: ProgramNodeStatus,
    },
    EmptyReason,
    InvalidProgram(String),
}

impl fmt::Display for ProgramReopenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNode(node_id) => write!(f, "unknown program node '{node_id}'"),
            Self::NodeNotPassed { node_id, status } => write!(
                f,
                "program node '{node_id}' cannot be reopened from status {status:?}"
            ),
            Self::EmptyReason => write!(f, "a retained regression reason is required"),
            Self::InvalidProgram(reason) => {
                write!(f, "reopen would leave invalid program: {reason}")
            }
        }
    }
}

impl std::error::Error for ProgramReopenError {}

impl ProgramDag {
    pub fn from_yaml(input: &str) -> Result<Self> {
        Ok(serde_yaml::from_str(input)?)
    }

    /// Validate graph shape, gates, lifecycle truth, and no-gap continuity.
    pub fn validate(&self) -> Result<ProgramFrontier> {
        if self.schema != 1 {
            bail!("unsupported program DAG schema {}; expected 1", self.schema);
        }
        if self.program_id.trim().is_empty() || self.objective.trim().is_empty() {
            bail!("program_id and objective are required");
        }
        if self.nodes.is_empty() || self.nodes.len() > 32 {
            bail!("program DAG must contain between 1 and 32 child DAGs");
        }

        let mut by_id = BTreeMap::new();
        for node in &self.nodes {
            if node.id.trim().is_empty() || by_id.insert(node.id.as_str(), node).is_some() {
                bail!("program node IDs must be non-empty and unique");
            }
            if node.child_dag.trim().is_empty()
                || node.worker_policy.trim().is_empty()
                || node.entry_gate.is_empty()
                || node.exit_gate.is_empty()
                || node.entry_gate.iter().any(|gate| gate.trim().is_empty())
                || node.exit_gate.iter().any(|gate| gate.trim().is_empty())
            {
                bail!(
                    "node '{}' needs a child DAG, worker policy, and non-empty entry/exit gates",
                    node.id
                );
            }
            if node.exit_gate.iter().collect::<BTreeSet<_>>().len() != node.exit_gate.len() {
                bail!("node '{}' repeats an exit gate", node.id);
            }
            match node.status {
                ProgramNodeStatus::Blocked
                    if node
                        .blocked_reason
                        .as_deref()
                        .is_none_or(|reason| reason.trim().is_empty()) =>
                {
                    bail!("blocked node '{}' needs a blocker", node.id);
                }
                ProgramNodeStatus::Blocked => {}
                _ if node.blocked_reason.is_some() => {
                    bail!("non-blocked node '{}' cannot claim a blocker", node.id);
                }
                _ => {}
            }
        }
        let Some(terminal) = by_id.get(self.terminal_node.as_str()) else {
            bail!("terminal_node '{}' does not exist", self.terminal_node);
        };
        if !terminal.next_on_pass.is_empty() {
            bail!("terminal node '{}' cannot have a successor", terminal.id);
        }

        let mut indegree: BTreeMap<&str, usize> = by_id.keys().map(|id| (*id, 0usize)).collect();
        let mut adjacency: BTreeMap<&str, Vec<&str>> =
            by_id.keys().map(|id| (*id, Vec::new())).collect();
        for node in &self.nodes {
            let mut seen_dependencies = BTreeSet::new();
            for dependency in &node.depends_on {
                if dependency == &node.id || !by_id.contains_key(dependency.as_str()) {
                    bail!(
                        "node '{}' has an invalid dependency '{}'",
                        node.id,
                        dependency
                    );
                }
                if !seen_dependencies.insert(dependency.as_str()) {
                    bail!("node '{}' repeats dependency '{}'", node.id, dependency);
                }
                *indegree.get_mut(node.id.as_str()).expect("known node") += 1;
                adjacency
                    .get_mut(dependency.as_str())
                    .expect("known dependency")
                    .push(node.id.as_str());
                if !by_id
                    .get(dependency.as_str())
                    .expect("known dependency")
                    .next_on_pass
                    .iter()
                    .any(|successor| successor == &node.id)
                {
                    bail!(
                        "dependency source '{}' must name '{}' as a successor",
                        dependency,
                        node.id
                    );
                }
            }
            let mut seen_successors = BTreeSet::new();
            for successor in &node.next_on_pass {
                let Some(target) = by_id.get(successor.as_str()) else {
                    bail!("node '{}' names unknown successor '{}'", node.id, successor);
                };
                if successor == &node.id || !seen_successors.insert(successor.as_str()) {
                    bail!(
                        "node '{}' has an invalid/repeated successor '{}'",
                        node.id,
                        successor
                    );
                }
                if !target
                    .depends_on
                    .iter()
                    .any(|dependency| dependency == &node.id)
                {
                    bail!(
                        "successor '{}' must depend on source '{}'",
                        successor,
                        node.id
                    );
                }
            }
            if node.id != self.terminal_node && node.next_on_pass.is_empty() {
                bail!("non-terminal node '{}' has no successor", node.id);
            }
        }

        let mut queue: VecDeque<&str> = indegree
            .iter()
            .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
            .collect();
        let mut visited = 0usize;
        while let Some(id) = queue.pop_front() {
            visited += 1;
            for successor in adjacency.get(id).expect("known node") {
                let degree = indegree.get_mut(successor).expect("known successor");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(successor);
                }
            }
        }
        if visited != self.nodes.len() {
            bail!("program DAG contains a dependency cycle");
        }

        // Every node must reach the declared terminal node through explicit
        // next_on_pass edges. Otherwise a locally successful child can strand
        // the program on a leaf that was never intended to be terminal.
        let mut reaches_terminal = BTreeSet::from([self.terminal_node.as_str()]);
        loop {
            let before = reaches_terminal.len();
            for node in &self.nodes {
                if node
                    .next_on_pass
                    .iter()
                    .any(|next| reaches_terminal.contains(next.as_str()))
                {
                    reaches_terminal.insert(node.id.as_str());
                }
            }
            if reaches_terminal.len() == before {
                break;
            }
        }
        if reaches_terminal.len() != self.nodes.len() {
            bail!(
                "every child DAG must lead to terminal node '{}'",
                self.terminal_node
            );
        }

        let passed: BTreeSet<&str> = self
            .nodes
            .iter()
            .filter(|node| node.status == ProgramNodeStatus::Passed)
            .map(|node| node.id.as_str())
            .collect();
        let dependencies_passed = |node: &ProgramNode| {
            node.depends_on
                .iter()
                .all(|dependency| passed.contains(dependency.as_str()))
        };
        for node in &self.nodes {
            if matches!(
                node.status,
                ProgramNodeStatus::Active | ProgramNodeStatus::Passed
            ) && !dependencies_passed(node)
            {
                bail!(
                    "active/passed node '{}' has an unpassed dependency",
                    node.id
                );
            }
        }
        let active = self
            .nodes
            .iter()
            .filter(|node| node.status == ProgramNodeStatus::Active)
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        let ready = self
            .nodes
            .iter()
            .filter(|node| node.status == ProgramNodeStatus::Planned && dependencies_passed(node))
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        let approval_required = self
            .nodes
            .iter()
            .filter(|node| {
                node.status == ProgramNodeStatus::Planned
                    && node.approval != ApprovalPolicy::None
                    && dependencies_passed(node)
            })
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        let blocked = self
            .nodes
            .iter()
            .filter(|node| node.status == ProgramNodeStatus::Blocked)
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        let complete = self
            .nodes
            .iter()
            .all(|node| node.status == ProgramNodeStatus::Passed);
        if !complete && active.is_empty() && ready.is_empty() && blocked.is_empty() {
            bail!("incomplete program has no active, ready, or explicitly blocked frontier");
        }
        Ok(ProgramFrontier {
            active,
            ready,
            approval_required,
            blocked,
            complete,
        })
    }

    /// Apply a complete set of explicit, passed exit-gate receipts to an
    /// active child and advance the manifest's lifecycle state atomically.
    ///
    /// This is intentionally a pure state transition over an in-memory
    /// manifest. It does not dispatch work, persist a store, or inspect child
    /// output. The existing roadmap remains responsible for execution.
    pub fn transition_active_node(
        &mut self,
        node_id: &str,
        receipts: &[ExitGateReceipt],
    ) -> std::result::Result<ProgramTransition, ProgramTransitionError> {
        self.validate()
            .map_err(|error| ProgramTransitionError::InvalidProgram(error.to_string()))?;
        let node = self
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| ProgramTransitionError::UnknownNode(node_id.to_owned()))?;
        if node.status != ProgramNodeStatus::Active {
            return Err(ProgramTransitionError::NodeNotActive {
                node_id: node_id.to_owned(),
                status: node.status,
            });
        }

        let expected: BTreeSet<&str> = node.exit_gate.iter().map(String::as_str).collect();
        let mut seen = BTreeSet::new();
        for receipt in receipts {
            if !expected.contains(receipt.gate.as_str()) {
                return Err(ProgramTransitionError::UnexpectedExitGate {
                    node_id: node_id.to_owned(),
                    gate: receipt.gate.clone(),
                });
            }
            if !seen.insert(receipt.gate.as_str()) {
                return Err(ProgramTransitionError::DuplicateExitReceipt {
                    node_id: node_id.to_owned(),
                    gate: receipt.gate.clone(),
                });
            }
            if !receipt.passed {
                return Err(ProgramTransitionError::ExitGateFailed {
                    node_id: node_id.to_owned(),
                    gate: receipt.gate.clone(),
                });
            }
        }
        if let Some(gate) = node
            .exit_gate
            .iter()
            .find(|gate| !seen.contains(gate.as_str()))
        {
            return Err(ProgramTransitionError::MissingExitGate {
                node_id: node_id.to_owned(),
                gate: gate.clone(),
            });
        }

        // Work against a clone so every failure leaves the caller's manifest
        // untouched. This also makes replay and invalid-frontier handling
        // deterministic for callers that persist the manifest themselves.
        let mut next = self.clone();
        let source_index = next
            .nodes
            .iter()
            .position(|node| node.id == node_id)
            .expect("validated node exists");
        next.nodes[source_index].status = ProgramNodeStatus::Passed;

        let passed: BTreeSet<String> = next
            .nodes
            .iter()
            .filter(|node| node.status == ProgramNodeStatus::Passed)
            .map(|node| node.id.clone())
            .collect();
        let successors = next.nodes[source_index].next_on_pass.clone();
        let mut activated = Vec::new();
        let mut approval_required = Vec::new();
        for successor_id in successors {
            let successor_index = next
                .nodes
                .iter()
                .position(|node| node.id == successor_id)
                .expect("validated successor exists");
            let successor = &mut next.nodes[successor_index];
            if successor.status != ProgramNodeStatus::Planned
                || !successor
                    .depends_on
                    .iter()
                    .all(|dependency| passed.contains(dependency.as_str()))
            {
                continue;
            }
            if successor.approval == ApprovalPolicy::None {
                successor.status = ProgramNodeStatus::Active;
                activated.push(successor.id.clone());
            } else {
                approval_required.push(successor.id.clone());
            }
        }
        activated.sort();
        approval_required.sort();

        let frontier = next
            .validate()
            .map_err(|error| ProgramTransitionError::InvalidProgram(error.to_string()))?;
        if !frontier.complete
            && frontier.active.is_empty()
            && frontier.ready.is_empty()
            && frontier.blocked.is_empty()
        {
            return Err(ProgramTransitionError::EmptyFrontier);
        }

        *self = next;
        Ok(ProgramTransition {
            node_id: node_id.to_owned(),
            activated,
            approval_required,
            frontier,
        })
    }

    /// Short alias for callers treating this as a generic program transition.
    pub fn transition_node(
        &mut self,
        node_id: &str,
        receipts: &[ExitGateReceipt],
    ) -> std::result::Result<ProgramTransition, ProgramTransitionError> {
        self.transition_active_node(node_id, receipts)
    }

    /// Atomically reactivate the earliest child DAG that owns a retained
    /// regression and reset all of its downstream descendants to `Planned`.
    ///
    /// Passed prerequisites and dependency-independent branches are preserved.
    /// Nodes with a human or spend approval return to an approval-ready planned
    /// state; approval-free nodes become active immediately. This is the
    /// machine-enforced back-edge used by the M0-M7 defect loop while keeping
    /// the declared program graph acyclic.
    pub fn reopen_for_regression(
        &mut self,
        node_id: &str,
        reason: &str,
    ) -> std::result::Result<ProgramReopen, ProgramReopenError> {
        self.validate()
            .map_err(|error| ProgramReopenError::InvalidProgram(error.to_string()))?;
        if reason.trim().is_empty() {
            return Err(ProgramReopenError::EmptyReason);
        }

        let node = self
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| ProgramReopenError::UnknownNode(node_id.to_owned()))?;
        if node.status != ProgramNodeStatus::Passed {
            return Err(ProgramReopenError::NodeNotPassed {
                node_id: node_id.to_owned(),
                status: node.status,
            });
        }

        let mut affected = BTreeSet::from([node_id.to_owned()]);
        let mut queue = VecDeque::from([node_id.to_owned()]);
        while let Some(current) = queue.pop_front() {
            let current_node = self
                .nodes
                .iter()
                .find(|candidate| candidate.id == current)
                .expect("validated node exists");
            for successor in &current_node.next_on_pass {
                if affected.insert(successor.clone()) {
                    queue.push_back(successor.clone());
                }
            }
        }

        let mut next = self.clone();
        let mut reset_descendants = Vec::new();
        let mut approval_required = false;
        for candidate in &mut next.nodes {
            if candidate.id == node_id {
                candidate.blocked_reason = None;
                if candidate.approval == ApprovalPolicy::None {
                    candidate.status = ProgramNodeStatus::Active;
                } else {
                    candidate.status = ProgramNodeStatus::Planned;
                    approval_required = true;
                }
            } else if affected.contains(&candidate.id) {
                candidate.status = ProgramNodeStatus::Planned;
                candidate.blocked_reason = None;
                reset_descendants.push(candidate.id.clone());
            }
        }
        reset_descendants.sort();

        let frontier = next
            .validate()
            .map_err(|error| ProgramReopenError::InvalidProgram(error.to_string()))?;
        *self = next;
        Ok(ProgramReopen {
            node_id: node_id.to_owned(),
            reason: reason.trim().to_owned(),
            reset_descendants,
            approval_required,
            frontier,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(status: ProgramNodeStatus) -> ProgramDag {
        ProgramDag {
            schema: 1,
            program_id: "p".into(),
            objective: "finish safely".into(),
            terminal_node: "release".into(),
            nodes: vec![
                ProgramNode {
                    id: "instrument".into(),
                    child_dag: "instrument.md".into(),
                    status,
                    depends_on: vec![],
                    next_on_pass: vec!["release".into()],
                    entry_gate: vec!["baseline".into()],
                    exit_gate: vec!["tests".into()],
                    worker_policy: "cheapest graduated".into(),
                    approval: ApprovalPolicy::None,
                    delivery: DeliveryMode::Code,
                    blocked_reason: None,
                },
                ProgramNode {
                    id: "release".into(),
                    child_dag: "release.md".into(),
                    status: ProgramNodeStatus::Planned,
                    depends_on: vec!["instrument".into()],
                    next_on_pass: vec![],
                    entry_gate: vec!["instrument passed".into()],
                    exit_gate: vec!["human approval".into()],
                    worker_policy: "integrator".into(),
                    approval: ApprovalPolicy::Human,
                    delivery: DeliveryMode::Code,
                    blocked_reason: None,
                },
            ],
        }
    }

    #[test]
    fn active_child_keeps_the_program_live() {
        let frontier = program(ProgramNodeStatus::Active).validate().unwrap();
        assert_eq!(frontier.active, ["instrument"]);
        assert!(frontier.ready.is_empty());
        assert!(!frontier.complete);
    }

    #[test]
    fn passing_a_child_exposes_its_successor_without_human_nudge() {
        let frontier = program(ProgramNodeStatus::Passed).validate().unwrap();
        assert_eq!(frontier.ready, ["release"]);
        assert_eq!(frontier.approval_required, ["release"]);
    }

    #[test]
    fn orphan_leaf_and_unpassed_active_dependency_are_rejected() {
        let mut orphan = program(ProgramNodeStatus::Active);
        orphan.nodes[0].next_on_pass.clear();
        assert!(orphan
            .validate()
            .unwrap_err()
            .to_string()
            .contains("no successor"));

        let mut impossible = program(ProgramNodeStatus::Planned);
        impossible.nodes[1].status = ProgramNodeStatus::Active;
        assert!(impossible
            .validate()
            .unwrap_err()
            .to_string()
            .contains("unpassed dependency"));
    }

    #[test]
    fn every_dependency_edge_requires_a_matching_successor_edge() {
        let mut dag = linear_program(ApprovalPolicy::None);
        dag.nodes.push(node(
            "side",
            ProgramNodeStatus::Passed,
            &[],
            &["finish"],
            ApprovalPolicy::None,
        ));
        dag.nodes[1].depends_on.push("side".into());
        dag.nodes[2].depends_on.push("side".into());

        assert!(dag
            .validate()
            .unwrap_err()
            .to_string()
            .contains("dependency source 'side' must name 'middle' as a successor"));
    }

    #[test]
    fn repository_master_programs_are_valid_and_have_active_frontiers() {
        let manifests = [
            (
                include_str!("../../../docs/orchestrator/CODING_HARNESS_MASTER_PROGRAM_DAG.yaml"),
                vec!["p4_task_budget_boundary"],
            ),
            (
                include_str!(
                    "../../../docs/orchestrator/VOICE_UX_RELIABILITY_MASTER_PROGRAM_DAG.yaml"
                ),
                vec!["v6_rebuild_device_e2e"],
            ),
            (
                include_str!("../../../docs/orchestrator/PERMAGENT_IMPROVEMENT_PORTFOLIO_DAG.yaml"),
                vec!["coding_harness_program", "voice_reliability_program"],
            ),
        ];
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (manifest, expected_active) in manifests {
            let program = ProgramDag::from_yaml(manifest).unwrap();
            let frontier = program.validate().unwrap();
            assert_eq!(frontier.active, expected_active);
            assert!(!frontier.complete);
            for node in &program.nodes {
                let path = node.child_dag.split('#').next().unwrap();
                assert!(
                    repository.join(path).is_file(),
                    "child DAG document is missing for {}: {}",
                    node.id,
                    node.child_dag
                );
            }
        }
    }

    fn node(
        id: &str,
        status: ProgramNodeStatus,
        depends_on: &[&str],
        next_on_pass: &[&str],
        approval: ApprovalPolicy,
    ) -> ProgramNode {
        ProgramNode {
            id: id.into(),
            child_dag: format!("{id}.md"),
            status,
            depends_on: depends_on.iter().map(|value| (*value).into()).collect(),
            next_on_pass: next_on_pass.iter().map(|value| (*value).into()).collect(),
            entry_gate: vec![format!("{id} entry")],
            exit_gate: vec![format!("{id} exit")],
            worker_policy: "deterministic worker".into(),
            approval,
            delivery: DeliveryMode::Code,
            blocked_reason: None,
        }
    }

    fn linear_program(successor_approval: ApprovalPolicy) -> ProgramDag {
        ProgramDag {
            schema: 1,
            program_id: "transition-tests".into(),
            objective: "test transitions".into(),
            terminal_node: "finish".into(),
            nodes: vec![
                node(
                    "start",
                    ProgramNodeStatus::Active,
                    &[],
                    &["middle"],
                    ApprovalPolicy::None,
                ),
                node(
                    "middle",
                    ProgramNodeStatus::Planned,
                    &["start"],
                    &["finish"],
                    successor_approval,
                ),
                node(
                    "finish",
                    ProgramNodeStatus::Planned,
                    &["middle"],
                    &[],
                    ApprovalPolicy::Human,
                ),
            ],
        }
    }

    #[test]
    fn transition_auto_promotes_ready_successor_without_a_frontier_gap() {
        let mut dag = linear_program(ApprovalPolicy::None);
        let transition = dag
            .transition_node("start", &[ExitGateReceipt::passed("start exit")])
            .unwrap();

        assert_eq!(dag.nodes[0].status, ProgramNodeStatus::Passed);
        assert_eq!(dag.nodes[1].status, ProgramNodeStatus::Active);
        assert_eq!(transition.activated, ["middle"]);
        assert!(transition.approval_required.is_empty());
        assert_eq!(transition.frontier.active, ["middle"]);
        assert!(transition.frontier.ready.is_empty());
    }

    #[test]
    fn transition_waits_for_every_fan_in_dependency() {
        let mut dag = ProgramDag {
            schema: 1,
            program_id: "fan-in-tests".into(),
            objective: "test fan-in".into(),
            terminal_node: "finish".into(),
            nodes: vec![
                node(
                    "left",
                    ProgramNodeStatus::Active,
                    &[],
                    &["merge"],
                    ApprovalPolicy::None,
                ),
                node(
                    "right",
                    ProgramNodeStatus::Active,
                    &[],
                    &["merge"],
                    ApprovalPolicy::None,
                ),
                node(
                    "merge",
                    ProgramNodeStatus::Planned,
                    &["left", "right"],
                    &["finish"],
                    ApprovalPolicy::None,
                ),
                node(
                    "finish",
                    ProgramNodeStatus::Planned,
                    &["merge"],
                    &[],
                    ApprovalPolicy::Human,
                ),
            ],
        };

        let first = dag
            .transition_active_node("left", &[ExitGateReceipt::passed("left exit")])
            .unwrap();
        assert!(first.activated.is_empty());
        assert_eq!(dag.nodes[2].status, ProgramNodeStatus::Planned);
        assert_eq!(first.frontier.active, ["right"]);

        let second = dag
            .transition_active_node("right", &[ExitGateReceipt::passed("right exit")])
            .unwrap();
        assert_eq!(second.activated, ["merge"]);
        assert_eq!(dag.nodes[2].status, ProgramNodeStatus::Active);
    }

    #[test]
    fn transition_rejects_replay_and_leaves_manifest_unchanged() {
        let mut dag = linear_program(ApprovalPolicy::None);
        let receipts = [ExitGateReceipt::passed("start exit")];
        dag.transition_active_node("start", &receipts).unwrap();
        let before = dag.clone();
        let error = dag.transition_active_node("start", &receipts).unwrap_err();

        assert!(matches!(
            error,
            ProgramTransitionError::NodeNotActive {
                status: ProgramNodeStatus::Passed,
                ..
            }
        ));
        assert_eq!(dag, before);
    }

    #[test]
    fn transition_rejects_missing_or_mismatched_exit_gates() {
        let mut dag = linear_program(ApprovalPolicy::None);
        dag.nodes[0].exit_gate.push("second exit".into());
        let before = dag.clone();
        let missing = dag
            .transition_active_node("start", &[ExitGateReceipt::passed("start exit")])
            .unwrap_err();
        assert!(matches!(
            missing,
            ProgramTransitionError::MissingExitGate { .. }
        ));
        assert_eq!(dag, before);

        let unexpected = dag
            .transition_active_node(
                "start",
                &[
                    ExitGateReceipt::passed("start exit"),
                    ExitGateReceipt::passed("second exit"),
                    ExitGateReceipt::passed("not declared"),
                ],
            )
            .unwrap_err();
        assert!(matches!(
            unexpected,
            ProgramTransitionError::UnexpectedExitGate { .. }
        ));
        assert_eq!(dag, before);
    }

    #[test]
    fn transition_surfaces_approval_boundary_without_auto_starting() {
        for approval in [ApprovalPolicy::Human, ApprovalPolicy::SpendCap] {
            let mut dag = linear_program(approval);
            let transition = dag
                .transition_active_node("start", &[ExitGateReceipt::passed("start exit")])
                .unwrap();

            assert!(transition.activated.is_empty());
            assert_eq!(transition.approval_required, ["middle"]);
            assert_eq!(dag.nodes[1].status, ProgramNodeStatus::Planned);
            assert_eq!(transition.frontier.approval_required, ["middle"]);
            assert!(transition.frontier.active.is_empty());
        }
    }

    #[test]
    fn retained_regression_reopens_owner_and_resets_every_descendant() {
        let mut dag = linear_program(ApprovalPolicy::None);
        dag.nodes[0].status = ProgramNodeStatus::Passed;
        dag.nodes[1].status = ProgramNodeStatus::Passed;
        dag.nodes[2].status = ProgramNodeStatus::Active;
        dag.validate().unwrap();

        let reopened = dag
            .reopen_for_regression("start", "held-out duplicate effect")
            .unwrap();

        assert_eq!(dag.nodes[0].status, ProgramNodeStatus::Active);
        assert_eq!(dag.nodes[1].status, ProgramNodeStatus::Planned);
        assert_eq!(dag.nodes[2].status, ProgramNodeStatus::Planned);
        assert_eq!(reopened.reset_descendants, ["finish", "middle"]);
        assert_eq!(reopened.frontier.active, ["start"]);
        assert!(!reopened.approval_required);
        assert_eq!(reopened.reason, "held-out duplicate effect");
    }

    #[test]
    fn reopening_an_approval_node_does_not_bypass_its_gate() {
        let mut dag = linear_program(ApprovalPolicy::Human);
        dag.nodes[0].status = ProgramNodeStatus::Passed;
        dag.nodes[1].status = ProgramNodeStatus::Passed;
        dag.validate().unwrap();

        let reopened = dag
            .reopen_for_regression("middle", "integrated trust regression")
            .unwrap();

        assert_eq!(dag.nodes[1].status, ProgramNodeStatus::Planned);
        assert_eq!(dag.nodes[2].status, ProgramNodeStatus::Planned);
        assert!(reopened.approval_required);
        assert_eq!(reopened.frontier.approval_required, ["middle"]);
        assert!(reopened.frontier.active.is_empty());
    }

    #[test]
    fn reopen_requires_a_reason_and_is_atomic_on_failure() {
        let mut dag = linear_program(ApprovalPolicy::None);
        dag.nodes[0].status = ProgramNodeStatus::Passed;
        dag.nodes[1].status = ProgramNodeStatus::Active;
        let before = dag.clone();

        assert_eq!(
            dag.reopen_for_regression("start", "   ").unwrap_err(),
            ProgramReopenError::EmptyReason
        );
        assert_eq!(dag, before);
    }
}
