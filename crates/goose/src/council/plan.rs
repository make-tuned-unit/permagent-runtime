//! Typed Council planning contract.
//!
//! This module is deliberately pure: deliberation may propose work, but only
//! the validated, explicitly approved plan returned by [`orchestrate`] can be
//! handed to a worker dispatcher.  Keeping validation here gives every caller
//! one deterministic graph/budget/capability/risk gate.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Component, Path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectContext {
    pub project_id: String,
    pub project_name: String,
    /// Snapshot assembled before deliberation; never inferred by a worker.
    pub summary: String,
    /// IDs of associated memories included in the snapshot.
    #[serde(default)]
    pub memory_ids: Vec<String>,
    /// The playbook briefing included in the snapshot, if the feature is on.
    #[serde(default)]
    pub playbook_briefing: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationSpec {
    pub command: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DagNode {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Exact repository-relative files this node may inspect or change.
    #[serde(default)]
    pub files: Vec<String>,
    /// Existing symbols that anchor the surgical change. Empty is allowed for
    /// a genuinely new file, but not as a substitute for `files`.
    #[serde(default)]
    pub symbols: Vec<String>,
    /// Existing components/modules/tests whose pattern the worker must follow.
    #[serde(default)]
    pub pattern_references: Vec<String>,
    /// Observable end-state checks, separate from the command that proves them.
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub required_capabilities: BTreeSet<String>,
    pub estimated_budget: u64,
    pub risk: RiskLevel,
    pub verification: VerificationSpec,
}

/// Immutable proposal/session input. Callers should construct a new proposal
/// for every revision rather than mutating one after validation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CouncilProposal {
    pub proposal_id: String,
    pub project: ProjectContext,
    pub nodes: Vec<DagNode>,
    pub budget_limit: u64,
    /// Identity of an optional, operator-supplied ProgramDag contract. The
    /// daemon computes this hash from the validated manifest; it is never
    /// accepted as a free-form approval label.
    #[serde(default)]
    pub program_manifest_sha256: Option<String>,
    /// The exact optional contract content shown in the approval payload.
    /// The hash is recomputed by the daemon and is the authority check.
    #[serde(default)]
    pub program_manifest: Option<String>,
}

/// The model-authored portion of a proposal. Project identity, memory ids and
/// the frozen brief are filled by the daemon, never trusted to the chair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CouncilDagDraft {
    pub nodes: Vec<DagNode>,
    pub budget_limit: u64,
    /// Optional exact ProgramDag YAML supplied with a Build request. When
    /// present, the daemon validates it and stores both the auditable content
    /// and its recomputable identity hash in the approval payload.
    #[serde(default)]
    pub program_manifest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub errors: Vec<ValidationError>,
    pub order: Vec<String>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalToken;

impl ApprovalToken {
    /// The UI/Decision Inbox is the authority that supplies this token.
    pub fn explicit() -> Self {
        Self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanSession {
    pub proposal: CouncilProposal,
    pub order: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    Invalid(ValidationReport),
    ApprovalRequired,
}

/// Deterministically validate a proposal against the capabilities available to
/// the authoritative worker router. No network, LLM, or filesystem access.
pub fn validate(proposal: &CouncilProposal, capabilities: &BTreeSet<String>) -> ValidationReport {
    let mut errors = Vec::new();
    if proposal.proposal_id.trim().is_empty() {
        errors.push(err("proposal_id", "proposal id is required"));
    }
    if proposal.project.project_id.trim().is_empty() {
        errors.push(err("project", "project id is required"));
    }
    if proposal.project.project_name.trim().is_empty() {
        errors.push(err("project", "project name is required"));
    }
    if proposal.project.summary.trim().is_empty() {
        errors.push(err(
            "context",
            "project context summary is required before deliberation",
        ));
    }
    if proposal.nodes.is_empty() {
        errors.push(err("nodes", "proposal must contain at least one DAG node"));
    }
    if proposal.nodes.len() > 12 {
        errors.push(err("nodes", "proposal may contain at most 12 DAG nodes"));
    }
    if proposal.budget_limit == 0 {
        errors.push(err("budget", "proposal budget limit must be positive"));
    }
    let mut ids = BTreeSet::new();
    let mut by_id = BTreeMap::new();
    for (index, node) in proposal.nodes.iter().enumerate() {
        if node.id.trim().is_empty() {
            errors.push(err("node_id", format!("node {index} has no id")));
        }
        if !ids.insert(node.id.clone()) {
            errors.push(err("node_id", format!("duplicate node id '{}'", node.id)));
        }
        by_id.insert(node.id.clone(), index);
        if node.title.trim().is_empty() {
            errors.push(err("node", format!("node '{}' has no title", node.id)));
        }
        if node.description.trim().is_empty() {
            errors.push(err(
                "node",
                format!("node '{}' has no implementation instructions", node.id),
            ));
        }
        if node.acceptance_criteria.is_empty()
            || node
                .acceptance_criteria
                .iter()
                .any(|criterion| criterion.trim().is_empty())
        {
            errors.push(err(
                "acceptance",
                format!("node '{}' needs explicit acceptance criteria", node.id),
            ));
        }
        if node
            .files
            .iter()
            .chain(node.symbols.iter())
            .chain(node.pattern_references.iter())
            .any(|value| value.trim().is_empty())
        {
            errors.push(err(
                "scope",
                format!("node '{}' contains a blank scope or pattern entry", node.id),
            ));
        }
        if node.files.iter().any(|file| !is_repo_relative(file)) {
            errors.push(err(
                "scope",
                format!(
                    "node '{}' files must be normalized repository-relative paths",
                    node.id
                ),
            ));
        }
        if node.required_capabilities.contains("code_edit") {
            if node.files.is_empty() {
                errors.push(err(
                    "scope",
                    format!("code-edit node '{}' needs exact file boundaries", node.id),
                ));
            }
            if node.pattern_references.is_empty() {
                errors.push(err(
                    "pattern",
                    format!(
                        "code-edit node '{}' needs an established-pattern reference",
                        node.id
                    ),
                ));
            }
            if !node.verification.required {
                errors.push(err(
                    "verification",
                    format!("code-edit node '{}' must require verification", node.id),
                ));
            }
        }
        if !(1..=10).contains(&node.estimated_budget) {
            errors.push(err(
                "budget",
                format!(
                    "node '{}' estimated budget must be a relative value from 1 to 10",
                    node.id
                ),
            ));
        }
        if node.verification.required && node.verification.command.trim().is_empty() {
            errors.push(err(
                "verification",
                format!("node '{}' requires a verification command", node.id),
            ));
        }
        if node.risk == RiskLevel::High && !node.verification.required {
            errors.push(err(
                "risk",
                format!("high-risk node '{}' must require verification", node.id),
            ));
        }
        let missing: Vec<_> = node
            .required_capabilities
            .difference(capabilities)
            .cloned()
            .collect();
        if !missing.is_empty() {
            errors.push(err(
                "capability",
                format!(
                    "node '{}' requires unavailable capabilities: {}",
                    node.id,
                    missing.join(", ")
                ),
            ));
        }
        for dep in &node.dependencies {
            if dep == &node.id {
                errors.push(err(
                    "dependency",
                    format!("node '{}' depends on itself", node.id),
                ));
            } else if !by_id.contains_key(dep) && !proposal.nodes.iter().any(|n| n.id == *dep) {
                errors.push(err(
                    "dependency",
                    format!("node '{}' depends on unknown node '{}'", node.id, dep),
                ));
            }
        }
    }
    let total: u64 = proposal.nodes.iter().map(|n| n.estimated_budget).sum();
    if total > proposal.budget_limit {
        errors.push(err(
            "budget",
            format!(
                "estimated budget {total} exceeds limit {}",
                proposal.budget_limit
            ),
        ));
    }

    // Stable Kahn ordering: BTreeMap/BTreeSet ensure identical proposals yield
    // identical dispatch order, independent of hash iteration order.
    let mut indegree: BTreeMap<String, usize> = ids.iter().map(|id| (id.clone(), 0)).collect();
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for node in &proposal.nodes {
        for dep in &node.dependencies {
            if ids.contains(dep) {
                *indegree.entry(node.id.clone()).or_default() += 1;
                edges
                    .entry(dep.clone())
                    .or_default()
                    .insert(node.id.clone());
            }
        }
    }
    let mut queue: VecDeque<String> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut order = Vec::with_capacity(ids.len());
    while let Some(id) = queue.pop_front() {
        order.push(id.clone());
        if let Some(children) = edges.get(&id) {
            for child in children {
                let degree = indegree.get_mut(child).expect("edge child is indexed");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(child.clone());
                }
            }
        }
    }
    if order.len() != ids.len() {
        errors.push(err("cycle", "dependency graph contains a cycle"));
    }
    ValidationReport { errors, order }
}

/// The sole transition from a proposal to a runnable plan session.
pub fn orchestrate(
    proposal: CouncilProposal,
    capabilities: &BTreeSet<String>,
    approval: Option<ApprovalToken>,
) -> Result<PlanSession, PlanError> {
    let report = validate(&proposal, capabilities);
    if !report.is_valid() {
        return Err(PlanError::Invalid(report));
    }
    if approval.is_none() {
        return Err(PlanError::ApprovalRequired);
    }
    Ok(PlanSession {
        order: report.order,
        proposal,
    })
}

/// Capabilities declared by the configured worker roster. Live availability
/// is checked again by the ordinary goal dispatcher; this set prevents a plan
/// from naming a capability no configured worker could ever provide.
pub fn configured_worker_capabilities() -> BTreeSet<String> {
    let config = crate::config::agent_identity::load_agent_config();
    worker_capabilities(config.workers.values())
}

fn worker_capabilities<'a>(
    workers: impl Iterator<Item = &'a crate::config::agent_identity::WorkerPersona>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for worker in workers {
        if !worker.engine.is_dispatchable() {
            continue;
        }
        out.extend(
            worker
                .tool_kinds
                .iter()
                .map(|kind| kind.trim().to_string())
                .filter(|kind| !kind.is_empty()),
        );
        let adapter = worker.capabilities();
        for (name, supported) in [
            ("model_override", adapter.supports_model_override),
            ("streaming", adapter.supports_streaming),
            ("steering", adapter.supports_steering),
            ("cancellation", adapter.supports_cancellation),
            ("sandbox", adapter.supports_sandbox),
            ("permission_gates", adapter.supports_permission_gates),
            ("mcp", adapter.supports_mcp),
            ("cli", adapter.supports_cli_tools),
        ] {
            if supported {
                out.insert(name.to_string());
            }
        }
    }
    out
}

fn err(code: &'static str, message: impl Into<String>) -> ValidationError {
    ValidationError {
        code,
        message: message.into(),
    }
}

fn is_repo_relative(value: &str) -> bool {
    let path = Path::new(value.trim());
    !value.trim().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, dependencies: &[&str]) -> DagNode {
        DagNode {
            id: id.into(),
            title: id.into(),
            description: "work".into(),
            files: vec!["src/example.rs".into()],
            symbols: vec!["example".into()],
            pattern_references: vec!["src/existing.rs::existing".into()],
            acceptance_criteria: vec!["requested behavior is observable".into()],
            dependencies: dependencies.iter().map(|s| (*s).into()).collect(),
            required_capabilities: ["code".into()].into(),
            estimated_budget: 2,
            risk: RiskLevel::Low,
            verification: VerificationSpec {
                command: "cargo test".into(),
                required: true,
            },
        }
    }
    fn proposal(nodes: Vec<DagNode>) -> CouncilProposal {
        CouncilProposal {
            proposal_id: "p-1".into(),
            project: ProjectContext {
                project_id: "project-1".into(),
                project_name: "Demo".into(),
                summary: "frozen context".into(),
                memory_ids: vec!["m-1".into()],
                playbook_briefing: Some("brief".into()),
            },
            nodes,
            budget_limit: 10,
            program_manifest_sha256: None,
            program_manifest: None,
        }
    }
    #[test]
    fn deterministic_order_and_explicit_approval_gate() {
        let p = proposal(vec![node("b", &["a"]), node("a", &[])]);
        let caps = ["code".into()].into();
        assert_eq!(validate(&p, &caps).order, vec!["a", "b"]);
        assert_eq!(
            orchestrate(p.clone(), &caps, None),
            Err(PlanError::ApprovalRequired)
        );
        assert_eq!(
            orchestrate(p, &caps, Some(ApprovalToken::explicit()))
                .unwrap()
                .order,
            vec!["a", "b"]
        );
    }
    #[test]
    fn rejects_cycles_missing_capabilities_and_budget() {
        let mut p = proposal(vec![node("a", &["b"]), node("b", &["a"])]);
        p.budget_limit = 1;
        let report = validate(&p, &BTreeSet::new());
        let codes: BTreeSet<_> = report.errors.iter().map(|e| e.code).collect();
        assert!(codes.contains("cycle"));
        assert!(codes.contains("capability"));
        assert!(codes.contains("budget"));
    }
    #[test]
    fn rejects_unknown_dependencies_and_missing_required_verification() {
        let mut n = node("a", &["missing"]);
        n.verification.command.clear();
        let report = validate(&proposal(vec![n]), &["code".into()].into());
        let codes: Vec<_> = report.errors.iter().map(|e| e.code).collect();
        assert!(codes.contains(&"dependency"));
        assert!(codes.contains(&"verification"));
    }

    #[test]
    fn code_edit_nodes_require_typed_scope_patterns_acceptance_and_verification() {
        let mut n = node("edit", &[]);
        n.required_capabilities = ["code_edit".into()].into();
        n.files.clear();
        n.pattern_references.clear();
        n.acceptance_criteria.clear();
        n.verification.required = false;
        let report = validate(&proposal(vec![n]), &["code_edit".into()].into());
        let codes: BTreeSet<_> = report.errors.iter().map(|error| error.code).collect();
        assert!(codes.contains("scope"));
        assert!(codes.contains("pattern"));
        assert!(codes.contains("acceptance"));
        assert!(codes.contains("verification"));

        let mut escaped = node("escaped", &[]);
        escaped.required_capabilities = ["code_edit".into()].into();
        escaped.files = vec!["../outside.rs".into()];
        let escaped_report = validate(&proposal(vec![escaped]), &["code_edit".into()].into());
        assert!(escaped_report
            .errors
            .iter()
            .any(|error| error.code == "scope"));
    }

    #[test]
    fn rejects_oversized_or_underspecified_plans() {
        let mut nodes: Vec<_> = (0..13)
            .map(|index| node(&format!("n-{index}"), &[]))
            .collect();
        nodes[0].description.clear();
        nodes[0].estimated_budget = 0;
        let mut p = proposal(nodes);
        p.budget_limit = 100;
        let report = validate(&p, &["code".into()].into());
        let messages = report
            .errors
            .iter()
            .map(|error| error.message.as_str())
            .collect::<Vec<_>>();
        assert!(messages
            .iter()
            .any(|message| message.contains("at most 12")));
        assert!(messages
            .iter()
            .any(|message| message.contains("implementation instructions")));
        assert!(messages
            .iter()
            .any(|message| message.contains("from 1 to 10")));
    }

    #[test]
    fn configured_capabilities_exclude_pending_only_workers() {
        let roster = crate::config::agent_identity::default_roster();
        let capabilities = worker_capabilities(roster.values());
        assert!(capabilities.contains("code_edit"));
        assert!(capabilities.contains("review"));
        assert!(!capabilities.contains("finance"));
        assert!(!capabilities.contains("memory_ops"));
    }
}
