//! Runtime handoff for the declarative program DAG.
//!
//! `permagent-eval` owns the pure manifest transition. This module is the
//! narrow bridge into the existing goal roadmap: it accepts a transition only
//! for a pre-mapped, already-complete goal, records an idempotency claim on
//! that goal, and reuses the normal dependency promotion/dispatch rail.
//!
//! It deliberately does not create cards, approve reviews, infer gates from
//! worker prose, or introduce a second scheduler.

use crate::agents::platform_extensions::{execution_receipt, goal_engine, orchestrator};
use crate::{cards, decisions, goal_state::GoalAction, goal_transition};
use permagent_eval::{
    DeliveryMode, ExitGateReceipt, ProgramDag, ProgramNodeStatus, ProgramTransition,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MAX_MANIFEST_CHARS: usize = 512_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramHandoffRequest {
    pub source_goal_id: String,
    pub node_id: String,
    pub manifest: String,
    pub receipts: Vec<ExitGateReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffStatus {
    Applied,
    AlreadyApplied,
    PendingDispatch,
    ApprovalRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramHandoffResponse {
    pub status: HandoffStatus,
    pub program_id: String,
    pub node_id: String,
    pub activated: Vec<String>,
    pub approval_required: Vec<String>,
    pub dispatched: u32,
    pub manifest: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ProgramHandoffError {
    #[error("invalid program handoff: {0}")]
    Invalid(String),
    #[error("program handoff conflict: {0}")]
    Conflict(String),
    #[error("program handoff pending: {0}")]
    Pending(String),
    #[error("program handoff storage failure: {0}")]
    Storage(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ProgramCardLink {
    program_id: String,
    node_id: String,
    manifest_sha256: String,
    /// Present for programs registered for automatic completion handoff.
    /// Older explicit CLI mappings may omit it and remain compatible.
    #[serde(default)]
    manifest: Option<String>,
    /// Explicitly approved research/audit nodes may complete without a git
    /// landing. Missing values retain the safe code-work default.
    #[serde(default = "default_delivery_mode")]
    delivery: DeliveryMode,
}

fn default_delivery_mode() -> DeliveryMode {
    DeliveryMode::Code
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramRegistrationRequest {
    pub manifest: String,
    /// Every manifest node must map to one already-existing goal card.
    pub node_to_goal: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramRegistrationResponse {
    pub program_id: String,
    pub manifest_sha256: String,
    pub mapped: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct TransitionRecord {
    digest: String,
    program_id: String,
    node_id: String,
    status: String,
    /// Retry state lives with the existing transition claim. Defaults keep
    /// records written before the maintenance retry rail compatible.
    #[serde(default)]
    attempt: u32,
    #[serde(default)]
    next_attempt_at: Option<i64>,
    #[serde(default)]
    last_error: Option<String>,
}

const RETRY_BASE_SECS: i64 = 5;
const RETRY_MAX_SECS: i64 = 30 * 60;

fn retry_delay_secs(attempt: u32) -> i64 {
    RETRY_BASE_SECS
        .saturating_mul(1_i64.checked_shl(attempt.min(16)).unwrap_or(i64::MAX))
        .min(RETRY_MAX_SECS)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum VerificationStatus {
    Pass,
    Fail,
    Uncertain,
}

/// Only the verdict's status is read through this type. The verification
/// identity is taken straight off the raw JSON in `load_registered_receipts`,
/// so carrying `finished_at` here as well left a field nothing ever read.
#[derive(Debug, Deserialize)]
struct StoredVerification {
    status: VerificationStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Claim {
    New,
    PendingRetry,
    AlreadyApplied,
}

fn manifest_digest(
    manifest: &str,
    program_id: &str,
    node_id: &str,
    receipts: &[ExitGateReceipt],
) -> String {
    let manifest_hash =
        manifest_identity_hash_from_yaml(manifest).unwrap_or_else(|_| manifest_hash(manifest));
    let input = serde_json::json!({
        "manifest_sha256": manifest_hash,
        "program_id": program_id,
        "node_id": node_id,
        "receipts": receipts,
    });
    let digest =
        Sha256::digest(serde_json::to_vec(&input).expect("handoff digest is serializable"));
    hex::encode(digest)
}

fn manifest_hash(manifest: &str) -> String {
    hex::encode(Sha256::digest(manifest.as_bytes()))
}

/// Reconstruct the already-claimed transition from the durable, transitioned
/// manifest used by `permagent-eval --daemon --in-place` retries. This path is
/// only valid when the source card already contains the matching
/// `program_transition` claim; a Passed node without that claim still fails
/// through `transition_active_node` below.
fn transition_from_claimed_manifest(
    program: &ProgramDag,
    node_id: &str,
) -> Result<ProgramTransition, String> {
    let node = program
        .nodes
        .iter()
        .find(|candidate| candidate.id == node_id)
        .ok_or_else(|| format!("unknown program node '{node_id}'"))?;
    if node.status != ProgramNodeStatus::Passed {
        return Err(format!(
            "claimed program node '{node_id}' is not Passed (status {:?})",
            node.status
        ));
    }

    let mut activated = Vec::new();
    let mut approval_required = Vec::new();
    for successor_id in &node.next_on_pass {
        let successor = program
            .nodes
            .iter()
            .find(|candidate| candidate.id == *successor_id)
            .ok_or_else(|| format!("unknown successor '{successor_id}'"))?;
        match (successor.status, successor.approval) {
            (ProgramNodeStatus::Active, permagent_eval::ApprovalPolicy::None) => {
                activated.push(successor.id.clone())
            }
            (
                ProgramNodeStatus::Planned,
                permagent_eval::ApprovalPolicy::Human | permagent_eval::ApprovalPolicy::SpendCap,
            ) => approval_required.push(successor.id.clone()),
            (ProgramNodeStatus::Active, _) => {
                return Err(format!(
                    "claimed approval-gated successor '{}' is Active",
                    successor.id
                ));
            }
            _ => {}
        }
    }
    activated.sort();
    approval_required.sort();
    let frontier = program.validate().map_err(|error| error.to_string())?;
    Ok(ProgramTransition {
        node_id: node_id.to_string(),
        activated,
        approval_required,
        frontier,
    })
}

/// Status fields change on every handoff; card mappings therefore bind to the
/// stable graph identity rather than to one particular lifecycle snapshot.
pub fn manifest_identity_hash(program: &ProgramDag) -> Result<String, String> {
    let mut identity = program.clone();
    for node in &mut identity.nodes {
        node.status = ProgramNodeStatus::Planned;
        node.blocked_reason = None;
    }
    let yaml = serde_yaml::to_string(&identity).map_err(|e| e.to_string())?;
    Ok(manifest_hash(&yaml))
}

fn manifest_identity_hash_from_yaml(manifest: &str) -> Result<String, String> {
    let program = ProgramDag::from_yaml(manifest).map_err(|e| e.to_string())?;
    manifest_identity_hash(&program)
}

fn link_from_card(card: &cards::Card) -> Result<ProgramCardLink, ProgramHandoffError> {
    serde_json::from_value(card.metadata_json.get("program").cloned().ok_or_else(|| {
        ProgramHandoffError::Invalid(format!("goal '{}' has no program mapping", card.id))
    })?)
    .map_err(|e| {
        ProgramHandoffError::Invalid(format!(
            "goal '{}' has invalid program mapping: {e}",
            card.id
        ))
    })
}

fn dependency_ids(card: &cards::Card) -> Result<Vec<String>, ProgramHandoffError> {
    let values = card
        .metadata_json
        .get("depends_on")
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            ProgramHandoffError::Invalid(format!(
                "goal '{}' has no typed depends_on mapping",
                card.id
            ))
        })?;
    let mut ids = values
        .iter()
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                ProgramHandoffError::Invalid(format!(
                    "goal '{}' has a non-string dependency",
                    card.id
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort();
    ids.dedup();
    if ids.len() != values.len() {
        return Err(ProgramHandoffError::Invalid(format!(
            "goal '{}' repeats a dependency",
            card.id
        )));
    }
    Ok(ids)
}

fn require_completed_evidence_metadata(
    metadata: &serde_json::Value,
    goal_id: &str,
) -> Result<(), ProgramHandoffError> {
    let receipt: execution_receipt::ExecutionReceipt =
        serde_json::from_value(metadata.get("execution_receipt").cloned().ok_or_else(|| {
            ProgramHandoffError::Invalid(format!("goal '{}' has no execution receipt", goal_id))
        })?)
        .map_err(|e| {
            ProgramHandoffError::Invalid(format!(
                "goal '{}' execution receipt is invalid: {e}",
                goal_id
            ))
        })?;
    if receipt.state != execution_receipt::ReceiptState::Completed {
        return Err(ProgramHandoffError::Invalid(format!(
            "goal '{}' execution receipt is {:?}, not Completed",
            goal_id, receipt.state
        )));
    }

    let evidence: goal_engine::GoalEvidence =
        serde_json::from_value(metadata.get("dispatch_evidence").cloned().ok_or_else(|| {
            ProgramHandoffError::Invalid(format!("goal '{}' has no dispatch evidence", goal_id))
        })?)
        .map_err(|e| {
            ProgramHandoffError::Invalid(format!(
                "goal '{}' dispatch evidence is invalid: {e}",
                goal_id
            ))
        })?;
    let verification_verdict = metadata
        .get("dispatch_evidence")
        .and_then(|evidence| evidence.get("verdict"))
        .cloned()
        .and_then(|value| serde_json::from_value::<StoredVerification>(value).ok());
    if !matches!(
        verification_verdict.map(|v| v.status),
        Some(VerificationStatus::Pass)
    ) || evidence.diff_errored
    {
        return Err(ProgramHandoffError::Invalid(format!(
            "goal '{}' lacks a typed passing verification evidence record",
            goal_id
        )));
    }
    Ok(())
}

fn require_completed_evidence(card: &cards::Card) -> Result<(), ProgramHandoffError> {
    require_completed_evidence_metadata(&card.metadata_json, &card.id)
}

fn require_no_write_evidence(
    metadata: &serde_json::Value,
    goal_id: &str,
) -> Result<(), ProgramHandoffError> {
    let evidence: goal_engine::GoalEvidence =
        serde_json::from_value(metadata.get("dispatch_evidence").cloned().ok_or_else(|| {
            ProgramHandoffError::Invalid(format!(
                "goal '{}' has no dispatch evidence for no-write proof",
                goal_id
            ))
        })?)
        .map_err(|error| {
            ProgramHandoffError::Invalid(format!(
                "goal '{}' no-write dispatch evidence is invalid: {error}",
                goal_id
            ))
        })?;
    if evidence.diff_errored
        || !evidence.commits.is_empty()
        || evidence.files_changed != 0
        || evidence.insertions != 0
        || evidence.deletions != 0
    {
        return Err(ProgramHandoffError::Invalid(format!(
            "goal '{}' no-write delivery requires trusted empty GoalEvidence",
            goal_id
        )));
    }
    Ok(())
}

/// Re-check the operator-approved manifest at consumption time. Registration
/// is not a permanent authorization if the Decision Inbox payload is later
/// replaced or the protected mapping is tampered with.
async fn require_current_approved_delivery(
    pool: &Pool<Sqlite>,
    source: &cards::Card,
    link: &ProgramCardLink,
    node: &permagent_eval::ProgramNode,
) -> Result<(), ProgramHandoffError> {
    let plan_id = source
        .metadata_json
        .get("council_plan_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ProgramHandoffError::Invalid(format!(
                "goal '{}' has no Council approval provenance",
                source.id
            ))
        })?;
    let payload: Option<String> = sqlx::query_scalar(
        "SELECT payload_json FROM decisions
          WHERE kind = 'council_action'
            AND status = 'answered'
            AND answer = 'approve'
            AND acted_by = ?
            AND project_id = ?
            AND json_extract(payload_json, '$.plan.proposal_id') = ?
          ORDER BY resolved_at DESC, id DESC LIMIT 1",
    )
    .bind(decisions::ACTOR_JESSE)
    .bind(&source.project_id)
    .bind(plan_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| ProgramHandoffError::Storage(error.to_string()))?;
    let payload = payload.ok_or_else(|| {
        ProgramHandoffError::Pending(format!(
            "Council approval '{}' is not currently available",
            plan_id
        ))
    })?;
    let payload: serde_json::Value = serde_json::from_str(&payload).map_err(|error| {
        ProgramHandoffError::Storage(format!("Council approval payload is invalid JSON: {error}"))
    })?;
    let plan = payload.get("plan").ok_or_else(|| {
        ProgramHandoffError::Invalid(format!("Council approval '{}' has no plan", plan_id))
    })?;
    let approved_hash = plan
        .get("program_manifest_sha256")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ProgramHandoffError::Invalid(format!(
                "Council approval '{}' has no manifest hash",
                plan_id
            ))
        })?;
    let approved_manifest = plan
        .get("program_manifest")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ProgramHandoffError::Invalid(format!(
                "Council approval '{}' has no manifest content",
                plan_id
            ))
        })?;
    let approved_program = ProgramDag::from_yaml(approved_manifest).map_err(|error| {
        ProgramHandoffError::Invalid(format!("approved Council manifest is invalid: {error}"))
    })?;
    approved_program.validate().map_err(|error| {
        ProgramHandoffError::Invalid(format!(
            "approved Council manifest failed validation: {error}"
        ))
    })?;
    let content_hash =
        manifest_identity_hash(&approved_program).map_err(ProgramHandoffError::Invalid)?;
    if approved_hash != content_hash || approved_hash != link.manifest_sha256 {
        return Err(ProgramHandoffError::Conflict(format!(
            "goal '{}' no longer matches the approved Council manifest",
            source.id
        )));
    }
    let approved_node = approved_program
        .nodes
        .iter()
        .find(|candidate| candidate.id == node.id)
        .ok_or_else(|| {
            ProgramHandoffError::Conflict(format!(
                "approved Council manifest has no node '{}'",
                node.id
            ))
        })?;
    if approved_node.delivery != node.delivery || node.delivery != link.delivery {
        return Err(ProgramHandoffError::Conflict(format!(
            "goal '{}' delivery mode is not the approved Council delivery mode",
            source.id
        )));
    }
    Ok(())
}

fn binding_is_terminal_success(binding: Option<&str>) -> bool {
    binding == Some("complete")
}

/// Register a declarative program against an existing roadmap without adding a
/// second persistence layer. Registration is intentionally all-or-nothing:
/// every manifest node maps to one existing goal in one project, dependency
/// edges match the roadmap, and a Passed node is backed by the same typed
/// completion evidence used by handoff.
pub async fn register_program(
    pool: &Pool<Sqlite>,
    request: ProgramRegistrationRequest,
) -> Result<ProgramRegistrationResponse, ProgramHandoffError> {
    if request.manifest.chars().count() > MAX_MANIFEST_CHARS {
        return Err(ProgramHandoffError::Invalid(
            "manifest exceeds size bound".to_string(),
        ));
    }
    let program = ProgramDag::from_yaml(&request.manifest)
        .map_err(|e| ProgramHandoffError::Invalid(format!("manifest is invalid: {e}")))?;
    program
        .validate()
        .map_err(|e| ProgramHandoffError::Invalid(format!("manifest is invalid: {e}")))?;
    if request.node_to_goal.len() != program.nodes.len() {
        return Err(ProgramHandoffError::Invalid(
            "every program node must map to exactly one existing goal".to_string(),
        ));
    }
    let expected_hash = manifest_identity_hash(&program).map_err(ProgramHandoffError::Invalid)?;
    let mut mapped: HashMap<String, cards::Card> = HashMap::new();
    let mut goal_ids = HashSet::new();
    for node in &program.nodes {
        let goal_id = request.node_to_goal.get(&node.id).ok_or_else(|| {
            ProgramHandoffError::Invalid(format!("program node '{}' is not mapped", node.id))
        })?;
        if goal_id.trim().is_empty() || !goal_ids.insert(goal_id.clone()) {
            return Err(ProgramHandoffError::Invalid(
                "program nodes must map to distinct non-empty goal IDs".to_string(),
            ));
        }
        let card = cards::get_card(pool, goal_id)
            .await
            .map_err(ProgramHandoffError::Storage)?
            .ok_or_else(|| {
                ProgramHandoffError::Invalid(format!("mapped goal '{}' was not found", goal_id))
            })?;
        if card.archived_at.is_some() || card.card_type != "goal" {
            return Err(ProgramHandoffError::Invalid(format!(
                "mapped card '{}' is not an active goal",
                goal_id
            )));
        }
        if let Some(existing) = mapped.values().next() {
            if existing.project_id != card.project_id {
                return Err(ProgramHandoffError::Conflict(
                    "program goals must belong to one project".to_string(),
                ));
            }
        }
        mapped.insert(node.id.clone(), card);
    }
    for node in &program.nodes {
        let card = mapped.get(&node.id).expect("validated node mapping");
        let expected_deps = node
            .depends_on
            .iter()
            .map(|dependency| {
                mapped
                    .get(dependency)
                    .map(|card| card.id.clone())
                    .ok_or_else(|| {
                        ProgramHandoffError::Invalid(format!(
                            "node '{}' depends on unmapped node '{}'",
                            node.id, dependency
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !sorted_equal(&dependency_ids(card)?, &expected_deps) {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' dependencies do not match manifest node '{}'",
                card.id, node.id
            )));
        }
        let column = cards::get_column(pool, &card.column_id)
            .await
            .map_err(ProgramHandoffError::Storage)?;
        let binding = column
            .as_ref()
            .and_then(|column| column.state_binding.as_deref());
        match node.status {
            ProgramNodeStatus::Passed => {
                if !binding_is_terminal_success(binding) {
                    return Err(ProgramHandoffError::Conflict(format!(
                        "Passed node '{}' is not a terminal-success goal",
                        node.id
                    )));
                }
                require_completed_evidence(card)?;
            }
            _ if binding_is_terminal_success(binding) => {
                return Err(ProgramHandoffError::Conflict(format!(
                    "goal '{}' is complete but manifest node '{}' is not Passed",
                    card.id, node.id
                )));
            }
            _ => {}
        }
        if let Some(existing) = card.metadata_json.get("program") {
            let existing: ProgramCardLink =
                serde_json::from_value(existing.clone()).map_err(|e| {
                    ProgramHandoffError::Conflict(format!(
                        "goal '{}' has an invalid existing program mapping: {e}",
                        card.id
                    ))
                })?;
            if existing.program_id != program.program_id
                || existing.node_id != node.id
                || existing.manifest_sha256 != expected_hash
                || existing
                    .manifest
                    .as_deref()
                    .is_some_and(|manifest| manifest != request.manifest)
                || existing.delivery != node.delivery
            {
                return Err(ProgramHandoffError::Conflict(format!(
                    "goal '{}' is already bound to a different program",
                    card.id
                )));
            }
        }
    }

    // Registration is not an approval mechanism. The first supported
    // provenance is an existing, answered Council DAG: its cards carry the
    // proposal id and the Decision Inbox must prove the user approved that exact
    // proposal for this project.
    let council_plan_id = mapped
        .values()
        .next()
        .and_then(|card| card.metadata_json.get("council_plan_id"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ProgramHandoffError::Invalid(
                "registration requires goals created by an approved Council DAG".to_string(),
            )
        })?
        .to_string();
    let mut council_node_ids = HashSet::new();
    for card in mapped.values() {
        let plan_id = card
            .metadata_json
            .get("council_plan_id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                ProgramHandoffError::Invalid(format!(
                    "goal '{}' has no Council approval provenance",
                    card.id
                ))
            })?;
        if plan_id != council_plan_id {
            return Err(ProgramHandoffError::Conflict(
                "all registered goals must belong to the same approved Council DAG".to_string(),
            ));
        }
        let node_id = card
            .metadata_json
            .get("council_node_id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                ProgramHandoffError::Invalid(format!(
                    "goal '{}' has no Council node provenance",
                    card.id
                ))
            })?;
        if !council_node_ids.insert(node_id.to_string()) {
            return Err(ProgramHandoffError::Conflict(
                "registered goals must have distinct Council node provenance".to_string(),
            ));
        }
    }
    let project_id = mapped
        .values()
        .next()
        .map(|card| card.project_id.clone())
        .ok_or_else(|| ProgramHandoffError::Invalid("program has no mapped goals".to_string()))?;
    let approved_payload: Option<String> = sqlx::query_scalar(
        "SELECT payload_json FROM decisions
          WHERE kind = 'council_action'
            AND status = 'answered'
            AND answer = 'approve'
            AND acted_by = ?
            AND project_id = ?
            AND json_extract(payload_json, '$.plan.proposal_id') = ?
          ORDER BY resolved_at DESC, id DESC LIMIT 1",
    )
    .bind(decisions::ACTOR_JESSE)
    .bind(&project_id)
    .bind(&council_plan_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let approved_payload = approved_payload.ok_or_else(|| {
        ProgramHandoffError::Invalid(format!(
            "Council plan '{}' has no answered approval for project '{}'",
            council_plan_id, project_id
        ))
    })?;
    let approved_payload: serde_json::Value =
        serde_json::from_str(&approved_payload).map_err(|e| {
            ProgramHandoffError::Storage(format!("Council approval payload is invalid JSON: {e}"))
        })?;
    let approved_manifest_hash = approved_payload
        .get("plan")
        .and_then(|plan| plan.get("program_manifest_sha256"))
        .and_then(|hash| hash.as_str())
        .filter(|hash| !hash.trim().is_empty())
        .ok_or_else(|| {
            ProgramHandoffError::Invalid(format!(
                "Council plan '{}' has no approved program manifest hash",
                council_plan_id
            ))
        })?;
    if approved_manifest_hash != expected_hash {
        return Err(ProgramHandoffError::Conflict(format!(
            "registered manifest does not match the approved Council manifest hash for '{}'",
            council_plan_id
        )));
    }
    let approved_manifest = approved_payload
        .get("plan")
        .and_then(|plan| plan.get("program_manifest"))
        .and_then(|manifest| manifest.as_str())
        .filter(|manifest| !manifest.trim().is_empty())
        .ok_or_else(|| {
            ProgramHandoffError::Invalid(format!(
                "Council plan '{}' has no approved program manifest content",
                council_plan_id
            ))
        })?;
    let approved_program = ProgramDag::from_yaml(approved_manifest).map_err(|error| {
        ProgramHandoffError::Invalid(format!(
            "approved Council program manifest is invalid: {error}"
        ))
    })?;
    let approved_content_hash =
        manifest_identity_hash(&approved_program).map_err(ProgramHandoffError::Invalid)?;
    if approved_content_hash != approved_manifest_hash {
        return Err(ProgramHandoffError::Conflict(format!(
            "approved Council manifest content does not match its recorded hash for '{}'",
            council_plan_id
        )));
    }
    let approved_node_ids = approved_payload
        .get("plan")
        .and_then(|plan| plan.get("nodes"))
        .and_then(|nodes| nodes.as_array())
        .ok_or_else(|| {
            ProgramHandoffError::Invalid(format!(
                "Council plan '{}' has no validated node list",
                council_plan_id
            ))
        })?
        .iter()
        .filter_map(|node| node.get("id").and_then(|id| id.as_str()))
        .collect::<HashSet<_>>();
    if council_node_ids
        .iter()
        .any(|node_id| !approved_node_ids.contains(node_id.as_str()))
    {
        return Err(ProgramHandoffError::Conflict(
            "registered goals are not all nodes of the approved Council DAG".to_string(),
        ));
    }

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let approved_payload_during_tx: Option<String> = sqlx::query_scalar(
        "SELECT payload_json FROM decisions
          WHERE kind = 'council_action'
            AND status = 'answered'
            AND answer = 'approve'
            AND acted_by = ?
            AND project_id = ?
            AND json_extract(payload_json, '$.plan.proposal_id') = ?
            AND json_extract(payload_json, '$.plan.program_manifest_sha256') = ?
          ORDER BY resolved_at DESC, id DESC LIMIT 1",
    )
    .bind(decisions::ACTOR_JESSE)
    .bind(&project_id)
    .bind(&council_plan_id)
    .bind(&expected_hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let approved_payload_during_tx = approved_payload_during_tx.ok_or_else(|| {
        ProgramHandoffError::Conflict("Council approval changed during registration".to_string())
    })?;
    let approved_payload_during_tx: serde_json::Value =
        serde_json::from_str(&approved_payload_during_tx).map_err(|error| {
            ProgramHandoffError::Storage(format!(
                "Council approval payload became invalid during registration: {error}"
            ))
        })?;
    let tx_manifest = approved_payload_during_tx
        .get("plan")
        .and_then(|plan| plan.get("program_manifest"))
        .and_then(|manifest| manifest.as_str())
        .ok_or_else(|| {
            ProgramHandoffError::Conflict(
                "Council approval lost its manifest content during registration".to_string(),
            )
        })?;
    let tx_program = ProgramDag::from_yaml(tx_manifest).map_err(|error| {
        ProgramHandoffError::Conflict(format!(
            "Council approval manifest became invalid during registration: {error}"
        ))
    })?;
    let tx_hash = manifest_identity_hash(&tx_program).map_err(ProgramHandoffError::Invalid)?;
    if tx_hash != expected_hash {
        return Err(ProgramHandoffError::Conflict(
            "Council approval changed during registration".to_string(),
        ));
    }
    let link_json = serde_json::json!({
        "program_id": program.program_id.clone(),
        "node_id": "",
        "manifest_sha256": expected_hash.clone(),
        "manifest": request.manifest.clone(),
        "delivery": DeliveryMode::Code,
    });
    for node in &program.nodes {
        let card = mapped.get(&node.id).expect("validated node mapping");
        let row = sqlx::query(
            "SELECT project_id, card_type, column_id, metadata_json FROM cards WHERE id = ? AND archived_at IS NULL",
        )
        .bind(&card.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?
        .ok_or_else(|| ProgramHandoffError::Conflict(format!("goal '{}' changed during registration", card.id)))?;
        let project_id: String = row.get("project_id");
        let card_type: String = row.get("card_type");
        if project_id != card.project_id || card_type != "goal" {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' changed during registration",
                card.id
            )));
        }
        let current_column_id: String = row.get("column_id");
        if current_column_id != card.column_id {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' changed lifecycle column during registration",
                card.id
            )));
        }
        let metadata_text: String = row.get("metadata_json");
        let mut metadata: serde_json::Value =
            serde_json::from_str(&metadata_text).map_err(|e| {
                ProgramHandoffError::Storage(format!("goal metadata is invalid JSON: {e}"))
            })?;
        if let Some(existing) = metadata.get("program") {
            let existing: ProgramCardLink =
                serde_json::from_value(existing.clone()).map_err(|e| {
                    ProgramHandoffError::Conflict(format!(
                        "goal '{}' has invalid program mapping: {e}",
                        card.id
                    ))
                })?;
            if existing.program_id != program.program_id
                || existing.node_id != node.id
                || existing.manifest_sha256 != expected_hash
                || existing
                    .manifest
                    .as_deref()
                    .is_some_and(|manifest| manifest != request.manifest)
                || existing.delivery != node.delivery
            {
                return Err(ProgramHandoffError::Conflict(format!(
                    "goal '{}' is already bound to a different program",
                    card.id
                )));
            }
        }
        if metadata.get("depends_on") != card.metadata_json.get("depends_on") {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' dependencies changed during registration",
                card.id
            )));
        }
        let current_plan_id = metadata
            .get("council_plan_id")
            .and_then(|value| value.as_str());
        if current_plan_id != Some(council_plan_id.as_str())
            || metadata.get("council_node_id") != card.metadata_json.get("council_node_id")
        {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' Council provenance changed during registration",
                card.id
            )));
        }
        let current_binding: Option<String> =
            sqlx::query_scalar("SELECT state_binding FROM board_columns WHERE id = ?")
                .bind(&current_column_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
        match node.status {
            ProgramNodeStatus::Passed => {
                if !binding_is_terminal_success(current_binding.as_deref()) {
                    return Err(ProgramHandoffError::Conflict(format!(
                        "Passed node '{}' changed out of terminal-success during registration",
                        node.id
                    )));
                }
                require_completed_evidence_metadata(&metadata, &card.id)?;
            }
            _ if binding_is_terminal_success(current_binding.as_deref()) => {
                return Err(ProgramHandoffError::Conflict(format!(
                    "goal '{}' became complete while node '{}' is not Passed",
                    card.id, node.id
                )));
            }
            _ => {}
        }
        let mut link = link_json.clone();
        link["node_id"] = serde_json::Value::String(node.id.clone());
        link["delivery"] = serde_json::to_value(node.delivery)
            .map_err(|error| ProgramHandoffError::Storage(error.to_string()))?;
        metadata["program"] = link;
        let updated = serde_json::to_string(&metadata)
            .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
        let changed = sqlx::query(
            "UPDATE cards SET metadata_json = ? WHERE id = ? AND metadata_json = ? AND archived_at IS NULL",
        )
        .bind(updated)
        .bind(&card.id)
        .bind(&metadata_text)
        .execute(&mut *tx)
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?
        .rows_affected();
        if changed != 1 {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' changed during registration",
                card.id
            )));
        }
    }
    decisions::append_audit_tx(
        &mut tx,
        "program_register",
        mapped.values().next().map(|card| card.id.as_str()),
        decisions::ACTOR_SYSTEM,
        0,
        "program:registered",
        Some(&expected_hash),
    )
    .await
    .map_err(ProgramHandoffError::Storage)?;
    tx.commit()
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;

    Ok(ProgramRegistrationResponse {
        program_id: program.program_id,
        manifest_sha256: expected_hash,
        mapped: request.node_to_goal,
    })
}

async fn claim_transition(
    pool: &Pool<Sqlite>,
    source_goal_id: &str,
    record: &TransitionRecord,
    expected_link: &ProgramCardLink,
    expected_manifest_hash: &str,
) -> Result<Claim, ProgramHandoffError> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let row = sqlx::query(
        "SELECT metadata_json, column_id, card_type FROM cards WHERE id = ? AND archived_at IS NULL",
    )
    .bind(source_goal_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?
    .ok_or_else(|| ProgramHandoffError::Invalid(format!("source goal '{}' was not found", source_goal_id)))?;
    let metadata_text: String = row.get("metadata_json");
    let column_id: String = row.get("column_id");
    let card_type: String = row.get("card_type");
    if card_type != "goal" {
        return Err(ProgramHandoffError::Invalid(format!(
            "source card '{}' is not a goal",
            source_goal_id
        )));
    }
    let binding: Option<String> =
        sqlx::query_scalar("SELECT state_binding FROM board_columns WHERE id = ?")
            .bind(&column_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    if !binding_is_terminal_success(binding.as_deref()) {
        return Err(ProgramHandoffError::Invalid(format!(
            "source goal '{}' is not terminal-success",
            source_goal_id
        )));
    }
    let mut metadata: serde_json::Value = serde_json::from_str(&metadata_text).map_err(|e| {
        ProgramHandoffError::Storage(format!("source metadata is invalid JSON: {e}"))
    })?;
    let link: ProgramCardLink =
        serde_json::from_value(metadata.get("program").cloned().ok_or_else(|| {
            ProgramHandoffError::Invalid("source program mapping disappeared".to_string())
        })?)
        .map_err(|e| {
            ProgramHandoffError::Storage(format!("source program mapping is invalid: {e}"))
        })?;
    if link.program_id != expected_link.program_id
        || link.node_id != expected_link.node_id
        || link.manifest_sha256 != expected_manifest_hash
    {
        return Err(ProgramHandoffError::Conflict(
            "source program mapping changed during handoff".to_string(),
        ));
    }
    require_completed_evidence_metadata(&metadata, source_goal_id)?;
    if let Some(existing) = metadata.get("program_transition") {
        let existing: TransitionRecord = serde_json::from_value(existing.clone()).map_err(|e| {
            ProgramHandoffError::Storage(format!("existing program transition is invalid: {e}"))
        })?;
        if existing.digest != record.digest {
            return Err(ProgramHandoffError::Conflict(format!(
                "source goal '{}' already has a different transition digest",
                source_goal_id
            )));
        }
        let claim = match existing.status.as_str() {
            "pending_dispatch" => Claim::PendingRetry,
            "dispatched" | "approval_required" => Claim::AlreadyApplied,
            other => {
                return Err(ProgramHandoffError::Storage(format!(
                    "unknown transition status '{other}'"
                )))
            }
        };
        tx.commit()
            .await
            .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
        return Ok(claim);
    }
    metadata["program_transition"] =
        serde_json::to_value(record).map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let updated = serde_json::to_string(&metadata)
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let changed =
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ? AND metadata_json = ?")
            .bind(updated)
            .bind(source_goal_id)
            .bind(&metadata_text)
            .execute(&mut *tx)
            .await
            .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?
            .rows_affected();
    if changed != 1 {
        return Err(ProgramHandoffError::Conflict(
            "source transition CAS lost a concurrent update".to_string(),
        ));
    }
    tx.commit()
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    Ok(Claim::New)
}

async fn finish_transition(
    pool: &Pool<Sqlite>,
    source_goal_id: &str,
    digest: &str,
    status: &str,
) -> Result<(), ProgramHandoffError> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let row = sqlx::query("SELECT metadata_json FROM cards WHERE id = ? AND archived_at IS NULL")
        .bind(source_goal_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?
        .ok_or_else(|| {
            ProgramHandoffError::Invalid(format!("source goal '{}' was not found", source_goal_id))
        })?;
    let text: String = row.get("metadata_json");
    let mut metadata: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        ProgramHandoffError::Storage(format!("source metadata is invalid JSON: {e}"))
    })?;
    let mut record: TransitionRecord =
        serde_json::from_value(metadata.get("program_transition").cloned().ok_or_else(|| {
            ProgramHandoffError::Conflict("transition claim disappeared".to_string())
        })?)
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    if record.digest != digest {
        return Err(ProgramHandoffError::Conflict(
            "transition digest changed during dispatch".to_string(),
        ));
    }
    record.status = status.to_string();
    if status == "pending_dispatch" {
        record.attempt = record.attempt.saturating_add(1);
        record.next_attempt_at = Some(unix_now().saturating_add(retry_delay_secs(record.attempt)));
        record.last_error = None;
    } else {
        record.next_attempt_at = None;
        record.last_error = None;
    }
    metadata["program_transition"] =
        serde_json::to_value(record).map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let updated = serde_json::to_string(&metadata)
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let changed =
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ? AND metadata_json = ?")
            .bind(updated)
            .bind(source_goal_id)
            .bind(text)
            .execute(&mut *tx)
            .await
            .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?
            .rows_affected();
    if changed != 1 {
        return Err(ProgramHandoffError::Conflict(
            "transition completion CAS lost a concurrent update".to_string(),
        ));
    }
    tx.commit()
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    Ok(())
}

/// Record a retryable or unexpected maintenance failure without changing the
/// authoritative transition status. This keeps a malformed/temporarily
/// unavailable claim from being retried on every maintenance tick while
/// preserving the durable claim for a later exact handoff.
async fn defer_transition_retry(
    pool: &Pool<Sqlite>,
    source_goal_id: &str,
    error: &str,
) -> Result<(), ProgramHandoffError> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let row = sqlx::query("SELECT metadata_json FROM cards WHERE id = ? AND archived_at IS NULL")
        .bind(source_goal_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?
        .ok_or_else(|| {
            ProgramHandoffError::Invalid(format!("source goal '{}' was not found", source_goal_id))
        })?;
    let text: String = row.get("metadata_json");
    let mut metadata: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        ProgramHandoffError::Storage(format!("source metadata is invalid JSON: {e}"))
    })?;
    let Some(value) = metadata.get_mut("program_transition") else {
        return Ok(());
    };
    let mut record: TransitionRecord = serde_json::from_value(value.clone())
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    if record.status != "pending_dispatch" {
        tx.commit()
            .await
            .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
        return Ok(());
    }
    record.attempt = record.attempt.saturating_add(1);
    record.next_attempt_at = Some(unix_now().saturating_add(retry_delay_secs(record.attempt)));
    record.last_error = Some(error.chars().take(512).collect());
    *value =
        serde_json::to_value(record).map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let updated = serde_json::to_string(&metadata)
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    let changed = sqlx::query(
        "UPDATE cards SET metadata_json = ? WHERE id = ? AND metadata_json = ? AND archived_at IS NULL",
    )
    .bind(updated)
    .bind(source_goal_id)
    .bind(text)
    .execute(&mut *tx)
    .await
    .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?
    .rows_affected();
    if changed != 1 {
        return Err(ProgramHandoffError::Conflict(
            "transition retry CAS lost a concurrent update".to_string(),
        ));
    }
    tx.commit()
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    Ok(())
}

fn sorted_equal(left: &[String], right: &[String]) -> bool {
    let mut a = left.to_vec();
    let mut b = right.to_vec();
    a.sort();
    b.sort();
    a == b
}

#[derive(Debug, Clone, Copy)]
struct ExactDispatchReport {
    processed: u32,
    dispatcher_installed: bool,
}

/// Promote and dispatch only the manifest's approval-free successors. This is
/// deliberately narrower than the normal completion nudge: a program
/// handoff must not cause an unrelated Ready goal, or an approval-gated
/// successor, to run as a side effect.
async fn dispatch_exact_successors(
    pool: &Pool<Sqlite>,
    project_id: &str,
    target_ids: &[String],
    dispatch: Option<&orchestrator::GoalDispatchHook>,
) -> Result<ExactDispatchReport, ProgramHandoffError> {
    let paused = crate::projects::list_tags(pool, project_id)
        .await
        .map_err(ProgramHandoffError::Storage)?
        .iter()
        .any(|tag| tag == "roadmap_paused");
    if paused {
        return Ok(ExactDispatchReport {
            processed: 0,
            dispatcher_installed: dispatch.is_some(),
        });
    }
    let mut processed = 0u32;
    for target_id in target_ids {
        let card = cards::get_card(pool, target_id)
            .await
            .map_err(ProgramHandoffError::Storage)?
            .ok_or_else(|| {
                ProgramHandoffError::Invalid(format!(
                    "mapped successor '{}' disappeared",
                    target_id
                ))
            })?;
        let column = cards::get_column(pool, &card.column_id)
            .await
            .map_err(ProgramHandoffError::Storage)?;
        let binding = column.as_ref().and_then(|c| c.state_binding.as_deref());
        if binding == Some("triage") {
            // The guard owns the lifecycle write; this explicit dependency
            // check prevents a malformed mapping from making a Triage card
            // Ready merely because this bridge was called.
            for dependency in dependency_ids(&card)? {
                let dep = cards::get_card(pool, &dependency)
                    .await
                    .map_err(ProgramHandoffError::Storage)?
                    .ok_or_else(|| {
                        ProgramHandoffError::Invalid(format!(
                            "dependency '{}' disappeared",
                            dependency
                        ))
                    })?;
                let dep_column = cards::get_column(pool, &dep.column_id)
                    .await
                    .map_err(ProgramHandoffError::Storage)?;
                if !binding_is_terminal_success(
                    dep_column.as_ref().and_then(|c| c.state_binding.as_deref()),
                ) {
                    return Ok(ExactDispatchReport {
                        processed,
                        dispatcher_installed: dispatch.is_some(),
                    });
                }
            }
            let ready_result = goal_transition::advance_goal_checked(
                pool,
                target_id,
                GoalAction::Ready,
                decisions::ACTOR_SYSTEM,
                None,
                goal_transition::TransitionEffects::default(),
            )
            .await;
            if let Err(error) = ready_result {
                // Two deliveries can observe Triage before either obtains the
                // lifecycle lock. If the competing writer has already moved
                // the card forward, treat that as the same idempotent
                // promotion; preserve every other lifecycle error.
                let refreshed = cards::get_card(pool, target_id)
                    .await
                    .map_err(ProgramHandoffError::Storage)?
                    .ok_or_else(|| {
                        ProgramHandoffError::Invalid(format!(
                            "mapped successor '{}' disappeared",
                            target_id
                        ))
                    })?;
                let refreshed_column = cards::get_column(pool, &refreshed.column_id)
                    .await
                    .map_err(ProgramHandoffError::Storage)?;
                let refreshed_binding = refreshed_column
                    .as_ref()
                    .and_then(|column| column.state_binding.as_deref());
                if !matches!(
                    refreshed_binding,
                    Some("ready" | "in_progress" | "review" | "complete")
                ) {
                    return Err(ProgramHandoffError::Pending(format!(
                        "successor '{}' could not become Ready: {error}",
                        target_id
                    )));
                }
            }
        }

        let current = cards::get_card(pool, target_id)
            .await
            .map_err(ProgramHandoffError::Storage)?
            .ok_or_else(|| {
                ProgramHandoffError::Invalid(format!(
                    "mapped successor '{}' disappeared",
                    target_id
                ))
            })?;
        let current_column = cards::get_column(pool, &current.column_id)
            .await
            .map_err(ProgramHandoffError::Storage)?;
        match current_column
            .as_ref()
            .and_then(|c| c.state_binding.as_deref())
        {
            Some("ready") => {
                let Some(dispatch) = dispatch else {
                    continue;
                };
                // A failed hook may have an unknown external side effect;
                // leave the transition pending instead of inferring success
                // from a concurrent lifecycle change.
                if dispatch(target_id.clone()).await.is_ok() {
                    processed += 1;
                }
            }
            Some("in_progress") | Some("review") | Some("complete") => processed += 1,
            _ => {}
        }
    }
    Ok(ExactDispatchReport {
        processed,
        dispatcher_installed: dispatch.is_some(),
    })
}

/// Validate and hand off one already-authorized child completion to the normal
/// goal roadmap. The manifest is never read from a caller-supplied path.
pub async fn apply_handoff(
    pool: &Pool<Sqlite>,
    request: ProgramHandoffRequest,
) -> Result<ProgramHandoffResponse, ProgramHandoffError> {
    apply_handoff_with_dispatch(pool, request, orchestrator::GOAL_DISPATCH_HOOK.get()).await
}

/// Persist the transitioned manifest back into the existing goal metadata for
/// a registered program. The CLI's `--in-place` file is not available to the
/// daemon completion path; without this update the next completed node would
/// keep seeing the original Active/Planned snapshot.
async fn persist_registered_manifest(
    pool: &Pool<Sqlite>,
    project_id: &str,
    program: &ProgramDag,
    rendered: &str,
) -> Result<(), ProgramHandoffError> {
    let expected_hash = manifest_identity_hash(program).map_err(ProgramHandoffError::Invalid)?;
    let cards = cards::list_cards(pool, project_id, Some("goal"), None)
        .await
        .map_err(ProgramHandoffError::Storage)?;
    let mapped = cards
        .into_iter()
        .filter_map(|card| {
            let link = link_from_card(&card).ok()?;
            (link.program_id == program.program_id
                && link.manifest_sha256 == expected_hash
                && program.nodes.iter().any(|node| node.id == link.node_id))
            .then_some(card)
        })
        .collect::<Vec<_>>();
    if mapped.len() != program.nodes.len() {
        return Err(ProgramHandoffError::Conflict(
            "registered program mapping is incomplete".to_string(),
        ));
    }
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    for card in mapped {
        let row = sqlx::query(
            "SELECT metadata_json, project_id, card_type FROM cards WHERE id = ? AND archived_at IS NULL",
        )
        .bind(&card.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?
        .ok_or_else(|| ProgramHandoffError::Conflict(format!("goal '{}' disappeared", card.id)))?;
        let project: String = row.get("project_id");
        let card_type: String = row.get("card_type");
        if project != project_id || card_type != "goal" {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' changed project/type",
                card.id
            )));
        }
        let old_text: String = row.get("metadata_json");
        let mut metadata: serde_json::Value = serde_json::from_str(&old_text).map_err(|e| {
            ProgramHandoffError::Storage(format!("goal metadata is invalid JSON: {e}"))
        })?;
        let Some(link_value) = metadata.get("program").cloned() else {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' lost program mapping",
                card.id
            )));
        };
        let link: ProgramCardLink = serde_json::from_value(link_value).map_err(|e| {
            ProgramHandoffError::Storage(format!(
                "goal '{}' program mapping is invalid: {e}",
                card.id
            ))
        })?;
        if link.program_id != program.program_id || link.manifest_sha256 != expected_hash {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' program mapping changed",
                card.id
            )));
        }
        let mut updated_link =
            serde_json::to_value(link).map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
        updated_link["manifest"] = serde_json::Value::String(rendered.to_string());
        metadata["program"] = updated_link;
        let updated = serde_json::to_string(&metadata)
            .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
        let changed = sqlx::query(
            "UPDATE cards SET metadata_json = ? WHERE id = ? AND metadata_json = ? AND archived_at IS NULL",
        )
        .bind(updated)
        .bind(&card.id)
        .bind(&old_text)
        .execute(&mut *tx)
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?
        .rows_affected();
        if changed != 1 {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' changed during manifest persistence",
                card.id
            )));
        }
    }
    tx.commit()
        .await
        .map_err(|e| ProgramHandoffError::Storage(e.to_string()))?;
    Ok(())
}

/// Read gate receipts persisted by the trusted verification path. Automatic
/// continuation never manufactures one passing receipt per declared gate.
fn load_registered_receipts(
    metadata: &serde_json::Value,
    node: &permagent_eval::ProgramNode,
    goal_id: &str,
) -> Result<Vec<ExitGateReceipt>, ProgramHandoffError> {
    let verification_id = metadata
        .get("dispatch_evidence")
        .and_then(|evidence| evidence.get("verdict"))
        .and_then(|verdict| verdict.get("finished_at"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ProgramHandoffError::Pending(format!(
                "goal '{}' has no current verification identity for gate receipts",
                goal_id
            ))
        })?;
    let value = metadata.get("program_receipts").cloned().ok_or_else(|| {
        ProgramHandoffError::Pending(format!(
            "goal '{}' has no persisted gate-specific program receipts",
            goal_id
        ))
    })?;
    let stored: Vec<ExitGateReceipt> = serde_json::from_value(value).map_err(|e| {
        ProgramHandoffError::Invalid(format!(
            "goal '{}' has invalid persisted program receipts: {e}",
            goal_id
        ))
    })?;
    let declared = node.exit_gate.iter().collect::<HashSet<_>>();
    let received = stored
        .iter()
        .map(|receipt| &receipt.gate)
        .collect::<HashSet<_>>();
    if declared.len() != received.len()
        || declared != received
        || stored.iter().any(|receipt| {
            !receipt.passed || receipt.verification_id.as_deref() != Some(verification_id)
        })
    {
        return Err(ProgramHandoffError::Pending(format!(
            "goal '{}' persisted program receipts do not prove every declared gate",
            goal_id
        )));
    }
    Ok(stored)
}

/// Automatic completion seam for a registered program. A normal goal
/// completion is allowed to invoke this only after the existing typed receipt,
/// dispatch evidence, verification verdict, and Complete lifecycle state have
/// all been established by the goal-transition/approval path.
pub async fn handoff_registered_goal(
    pool: &Pool<Sqlite>,
    goal_id: &str,
) -> Result<Option<ProgramHandoffResponse>, ProgramHandoffError> {
    handoff_registered_goal_with_dispatch(pool, goal_id, orchestrator::GOAL_DISPATCH_HOOK.get())
        .await
}

/// Retry one durable `pending_dispatch` claim during boot reconciliation.
///
/// This is deliberately a thin wrapper around the exact registered handoff
/// seam. It does not inspect or promote generic Ready cards, and it returns
/// `None` when another writer has already completed the transition.
pub(crate) async fn retry_pending_registered_goal_with_dispatch(
    pool: &Pool<Sqlite>,
    goal_id: &str,
    dispatch: Option<&orchestrator::GoalDispatchHook>,
) -> Result<Option<ProgramHandoffResponse>, ProgramHandoffError> {
    let Some(source) = cards::get_card(pool, goal_id)
        .await
        .map_err(ProgramHandoffError::Storage)?
    else {
        return Ok(None);
    };
    if source.archived_at.is_some()
        || source
            .metadata_json
            .get("program_transition")
            .and_then(|value| value.get("status"))
            .and_then(serde_json::Value::as_str)
            != Some("pending_dispatch")
    {
        return Ok(None);
    }
    match handoff_registered_goal_with_dispatch(pool, goal_id, dispatch).await {
        Ok(response) => Ok(response),
        Err(error) => {
            // Keep the claim durable but make the next maintenance attempt
            // due-bound and observable. The original error remains the
            // caller-facing result; retry bookkeeping is best effort and
            // never turns a fail-closed handoff into success.
            let _ = defer_transition_retry(pool, goal_id, &error.to_string()).await;
            Err(error)
        }
    }
}

async fn handoff_registered_goal_with_dispatch(
    pool: &Pool<Sqlite>,
    goal_id: &str,
    dispatch: Option<&orchestrator::GoalDispatchHook>,
) -> Result<Option<ProgramHandoffResponse>, ProgramHandoffError> {
    let source = cards::get_card(pool, goal_id)
        .await
        .map_err(ProgramHandoffError::Storage)?;
    let Some(source) = source else {
        return Ok(None);
    };
    let Some(link_value) = source.metadata_json.get("program") else {
        return Ok(None);
    };
    let link: ProgramCardLink = serde_json::from_value(link_value.clone()).map_err(|e| {
        ProgramHandoffError::Invalid(format!(
            "goal '{}' has invalid program mapping: {e}",
            goal_id
        ))
    })?;
    // Explicit CLI mappings from before registration remain valid, but they
    // are not eligible for the automatic completion hook because their
    // authoritative manifest still lives with the caller's in-place file.
    let Some(manifest) = link.manifest.clone() else {
        return Ok(None);
    };
    let program = ProgramDag::from_yaml(&manifest).map_err(|e| {
        ProgramHandoffError::Invalid(format!("registered manifest is invalid: {e}"))
    })?;
    program.validate().map_err(|e| {
        ProgramHandoffError::Invalid(format!("registered manifest is invalid: {e}"))
    })?;
    let node = program
        .nodes
        .iter()
        .find(|node| node.id == link.node_id)
        .ok_or_else(|| {
            ProgramHandoffError::Conflict("registered node is absent from manifest".to_string())
        })?;
    if node.status == ProgramNodeStatus::Passed
        && source
            .metadata_json
            .get("program_transition")
            .and_then(|value| value.get("digest"))
            .is_none()
    {
        return Err(ProgramHandoffError::Invalid(format!(
            "registered source node '{}' is Passed without a durable transition claim",
            node.id
        )));
    }
    if !matches!(
        node.status,
        ProgramNodeStatus::Active | ProgramNodeStatus::Passed
    ) {
        return Err(ProgramHandoffError::Invalid(format!(
            "registered source node '{}' is not Active or a claimed Passed replay",
            node.id
        )));
    }
    require_current_approved_delivery(pool, &source, &link, node).await?;
    require_completed_evidence(&source)?;
    if node.delivery == DeliveryMode::NoWrite {
        require_no_write_evidence(&source.metadata_json, goal_id)?;
    }
    let receipts = load_registered_receipts(&source.metadata_json, node, goal_id)?;
    let response = apply_handoff_with_registered_dispatch(
        pool,
        ProgramHandoffRequest {
            source_goal_id: source.id.clone(),
            node_id: node.id.clone(),
            manifest,
            receipts,
        },
        dispatch,
    )
    .await?;
    Ok(Some(response))
}

/// Testable seam for the process-global dispatch hook. Production callers use
/// [`apply_handoff`]; tests inject a recorder or `None` to prove pending and
/// paused behavior without mutating the OnceLock shared by other tests.
async fn apply_handoff_with_dispatch(
    pool: &Pool<Sqlite>,
    request: ProgramHandoffRequest,
    dispatch: Option<&orchestrator::GoalDispatchHook>,
) -> Result<ProgramHandoffResponse, ProgramHandoffError> {
    apply_handoff_with_dispatch_mode(pool, request, dispatch, false).await
}

async fn apply_handoff_with_registered_dispatch(
    pool: &Pool<Sqlite>,
    request: ProgramHandoffRequest,
    dispatch: Option<&orchestrator::GoalDispatchHook>,
) -> Result<ProgramHandoffResponse, ProgramHandoffError> {
    apply_handoff_with_dispatch_mode(pool, request, dispatch, true).await
}

async fn apply_handoff_with_dispatch_mode(
    pool: &Pool<Sqlite>,
    request: ProgramHandoffRequest,
    dispatch: Option<&orchestrator::GoalDispatchHook>,
    persist_before_dispatch: bool,
) -> Result<ProgramHandoffResponse, ProgramHandoffError> {
    if request.manifest.chars().count() > MAX_MANIFEST_CHARS {
        return Err(ProgramHandoffError::Invalid(
            "manifest exceeds size bound".to_string(),
        ));
    }
    if request.source_goal_id.trim().is_empty() || request.node_id.trim().is_empty() {
        return Err(ProgramHandoffError::Invalid(
            "source_goal_id and node_id are required".to_string(),
        ));
    }
    let mut program = ProgramDag::from_yaml(&request.manifest)
        .map_err(|e| ProgramHandoffError::Invalid(format!("manifest is invalid: {e}")))?;
    let source = cards::get_card(pool, &request.source_goal_id)
        .await
        .map_err(ProgramHandoffError::Storage)?
        .ok_or_else(|| {
            ProgramHandoffError::Invalid(format!(
                "source goal '{}' was not found",
                request.source_goal_id
            ))
        })?;
    let source_link = link_from_card(&source)?;
    let expected_hash = manifest_identity_hash(&program).map_err(ProgramHandoffError::Invalid)?;
    if source_link.manifest_sha256 != expected_hash {
        return Err(ProgramHandoffError::Conflict(
            "manifest hash does not match source goal mapping".to_string(),
        ));
    }
    if source_link.node_id != request.node_id || source_link.program_id != program.program_id {
        return Err(ProgramHandoffError::Conflict(
            "source goal mapping does not match manifest node".to_string(),
        ));
    }
    let source_column = cards::get_column(pool, &source.column_id)
        .await
        .map_err(ProgramHandoffError::Storage)?;
    if !binding_is_terminal_success(
        source_column
            .as_ref()
            .and_then(|c| c.state_binding.as_deref()),
    ) {
        return Err(ProgramHandoffError::Invalid(
            "source goal is not terminal-success".to_string(),
        ));
    }
    require_completed_evidence(&source)?;

    let digest = manifest_digest(
        &request.manifest,
        &program.program_id,
        &request.node_id,
        &request.receipts,
    );
    let existing_record = source
        .metadata_json
        .get("program_transition")
        .and_then(|value| serde_json::from_value::<TransitionRecord>(value.clone()).ok())
        .filter(|record| record.digest == digest);
    let source_was_passed = program
        .nodes
        .iter()
        .find(|node| node.id == request.node_id)
        .is_some_and(|node| node.status == ProgramNodeStatus::Passed);
    let transition = if existing_record.is_some() && source_was_passed {
        transition_from_claimed_manifest(&program, &request.node_id)
            .map_err(ProgramHandoffError::Invalid)?
    } else {
        program
            .transition_active_node(&request.node_id, &request.receipts)
            .map_err(|e| ProgramHandoffError::Invalid(e.to_string()))?
    };
    let record = TransitionRecord {
        digest: digest.clone(),
        program_id: program.program_id.clone(),
        node_id: request.node_id.clone(),
        status: "pending_dispatch".to_string(),
        attempt: 0,
        next_attempt_at: None,
        last_error: None,
    };

    let all_cards = cards::list_cards(pool, &source.project_id, Some("goal"), None)
        .await
        .map_err(ProgramHandoffError::Storage)?;
    let mut mapped: HashMap<String, cards::Card> = HashMap::new();
    for card in all_cards {
        let Ok(link) = link_from_card(&card) else {
            continue;
        };
        if link.program_id != program.program_id {
            continue;
        }
        if link.manifest_sha256 != expected_hash {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' uses a different program manifest",
                card.id
            )));
        }
        if mapped.insert(link.node_id.clone(), card).is_some() {
            return Err(ProgramHandoffError::Conflict(
                "program has duplicate mapped goal nodes".to_string(),
            ));
        }
    }
    if mapped.get(&request.node_id).map(|c| c.id.as_str()) != Some(source.id.as_str()) {
        return Err(ProgramHandoffError::Conflict(
            "source node is not mapped to the supplied goal".to_string(),
        ));
    }
    for node in &program.nodes {
        let Some(card) = mapped.get(&node.id) else {
            continue;
        };
        let expected_deps = node
            .depends_on
            .iter()
            .map(|dependency| {
                mapped
                    .get(dependency)
                    .map(|card| card.id.clone())
                    .ok_or_else(|| {
                        ProgramHandoffError::Invalid(format!(
                            "mapped node '{}' has an unmapped dependency '{}'",
                            node.id, dependency
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !sorted_equal(&dependency_ids(card)?, &expected_deps) {
            return Err(ProgramHandoffError::Conflict(format!(
                "goal '{}' dependencies do not match manifest node '{}'",
                card.id, node.id
            )));
        }
    }
    if existing_record.is_none() {
        for node_id in transition
            .activated
            .iter()
            .chain(transition.approval_required.iter())
        {
            let card = mapped.get(node_id).ok_or_else(|| {
                ProgramHandoffError::Invalid(format!(
                    "successor node '{}' has no authorized mapped goal",
                    node_id
                ))
            })?;
            let column = cards::get_column(pool, &card.column_id)
                .await
                .map_err(ProgramHandoffError::Storage)?;
            if column.as_ref().and_then(|c| c.state_binding.as_deref()) == Some("complete") {
                return Err(ProgramHandoffError::Conflict(format!(
                    "successor goal '{}' is already complete",
                    card.id
                )));
            }
        }
    }

    let claim = claim_transition(pool, &source.id, &record, &source_link, &expected_hash).await?;
    let rendered = serde_yaml::to_string(&program).map_err(|e| {
        ProgramHandoffError::Invalid(format!("transitioned manifest is not serializable: {e}"))
    })?;
    if claim == Claim::AlreadyApplied {
        let status = source
            .metadata_json
            .get("program_transition")
            .and_then(|v| v.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("dispatched");
        return Ok(ProgramHandoffResponse {
            status: if status == "approval_required" {
                HandoffStatus::ApprovalRequired
            } else {
                HandoffStatus::AlreadyApplied
            },
            program_id: program.program_id,
            node_id: request.node_id,
            activated: transition.activated,
            approval_required: transition.approval_required,
            dispatched: 0,
            manifest: rendered,
        });
    }

    if persist_before_dispatch {
        persist_registered_manifest(pool, &source.project_id, &program, &rendered).await?;
    }

    let target_ids = transition
        .activated
        .iter()
        .map(|node_id| {
            mapped
                .get(node_id)
                .map(|card| card.id.clone())
                .ok_or_else(|| {
                    ProgramHandoffError::Invalid(format!(
                        "successor node '{}' has no authorized mapped goal",
                        node_id
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let report = dispatch_exact_successors(pool, &source.project_id, &target_ids, dispatch).await?;
    let automatic_pending = !transition.activated.is_empty()
        && (!report.dispatcher_installed || report.processed < transition.activated.len() as u32);
    let status = if automatic_pending {
        "pending_dispatch"
    } else if !transition.approval_required.is_empty() {
        "approval_required"
    } else {
        "dispatched"
    };
    finish_transition(pool, &source.id, &digest, status).await?;
    let response_status = match status {
        "pending_dispatch" => HandoffStatus::PendingDispatch,
        "approval_required" => HandoffStatus::ApprovalRequired,
        _ => HandoffStatus::Applied,
    };
    Ok(ProgramHandoffResponse {
        status: response_status,
        program_id: program.program_id,
        node_id: request.node_id,
        activated: transition.activated,
        approval_required: transition.approval_required,
        dispatched: report.processed,
        manifest: rendered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decisions::{answer_decision, create_decision, DecisionAnswer, NewDecision};
    use permagent_eval::{ApprovalPolicy, DeliveryMode, ProgramNode};
    use std::sync::{Arc, Mutex};

    fn program(status: ProgramNodeStatus) -> ProgramDag {
        ProgramDag {
            schema: 1,
            program_id: "identity-test".to_string(),
            objective: "test stable graph identity".to_string(),
            terminal_node: "finish".to_string(),
            nodes: vec![
                ProgramNode {
                    id: "start".to_string(),
                    child_dag: "start.md".to_string(),
                    status,
                    depends_on: Vec::new(),
                    next_on_pass: vec!["finish".to_string()],
                    entry_gate: vec!["ready".to_string()],
                    exit_gate: vec!["checks".to_string()],
                    worker_policy: "cheap".to_string(),
                    approval: ApprovalPolicy::None,
                    delivery: DeliveryMode::Code,
                    blocked_reason: None,
                },
                ProgramNode {
                    id: "finish".to_string(),
                    child_dag: "finish.md".to_string(),
                    status: ProgramNodeStatus::Planned,
                    depends_on: vec!["start".to_string()],
                    next_on_pass: Vec::new(),
                    entry_gate: vec!["start passed".to_string()],
                    exit_gate: vec!["approved".to_string()],
                    worker_policy: "integrator".to_string(),
                    approval: ApprovalPolicy::Human,
                    delivery: DeliveryMode::Code,
                    blocked_reason: None,
                },
            ],
        }
    }

    #[test]
    fn identity_hash_ignores_lifecycle_status() {
        assert_eq!(
            manifest_identity_hash(&program(ProgramNodeStatus::Active)).unwrap(),
            manifest_identity_hash(&program(ProgramNodeStatus::Passed)).unwrap()
        );
    }

    #[test]
    fn completion_authority_requires_typed_receipt_evidence_and_verdict() {
        let mut receipt = execution_receipt::ExecutionReceipt::new(
            "worker",
            "session",
            serde_json::json!({}),
            "lifecycle",
            "2026-09-05T00:00:00Z",
            1,
        );
        receipt.finalize(
            execution_receipt::ReceiptState::Completed,
            "2026-09-05T00:01:00Z",
        );
        let mut evidence = serde_json::to_value(goal_engine::GoalEvidence::default()).unwrap();
        evidence["verdict"] = serde_json::json!({"status": "pass"});
        let metadata = serde_json::json!({
            "execution_receipt": receipt,
            "dispatch_evidence": evidence
        });
        require_completed_evidence_metadata(&metadata, "goal").unwrap();

        let mut failed = metadata.clone();
        failed["dispatch_evidence"]["verdict"]["status"] = serde_json::json!("uncertain");
        assert!(require_completed_evidence_metadata(&failed, "goal").is_err());
    }

    fn chain_program() -> ProgramDag {
        ProgramDag {
            schema: 1,
            program_id: "handoff-chain".to_string(),
            objective: "exercise the database handoff rail".to_string(),
            terminal_node: "c".to_string(),
            nodes: vec![
                ProgramNode {
                    id: "a".to_string(),
                    child_dag: "a.md".to_string(),
                    status: ProgramNodeStatus::Active,
                    depends_on: Vec::new(),
                    next_on_pass: vec!["b".to_string()],
                    entry_gate: vec!["start".to_string()],
                    exit_gate: vec!["a done".to_string()],
                    worker_policy: "cheap".to_string(),
                    approval: ApprovalPolicy::None,
                    delivery: DeliveryMode::Code,
                    blocked_reason: None,
                },
                ProgramNode {
                    id: "b".to_string(),
                    child_dag: "b.md".to_string(),
                    status: ProgramNodeStatus::Planned,
                    depends_on: vec!["a".to_string()],
                    next_on_pass: vec!["c".to_string()],
                    entry_gate: vec!["a passed".to_string()],
                    exit_gate: vec!["b done".to_string()],
                    worker_policy: "cheap".to_string(),
                    approval: ApprovalPolicy::None,
                    delivery: DeliveryMode::Code,
                    blocked_reason: None,
                },
                ProgramNode {
                    id: "c".to_string(),
                    child_dag: "c.md".to_string(),
                    status: ProgramNodeStatus::Planned,
                    depends_on: vec!["b".to_string()],
                    next_on_pass: Vec::new(),
                    entry_gate: vec!["b passed".to_string()],
                    exit_gate: vec!["c done".to_string()],
                    worker_policy: "cheap".to_string(),
                    approval: ApprovalPolicy::None,
                    delivery: DeliveryMode::Code,
                    blocked_reason: None,
                },
            ],
        }
    }

    async fn bridge_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn mapped_goal(
        pool: &Pool<Sqlite>,
        node_id: &str,
        binding: &str,
        dependencies: &[String],
        program: &ProgramDag,
    ) -> cards::Card {
        use crate::projects::PERSONAL_PROJECT_ID;
        cards::seed_goal_columns(pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let column = cards::get_goal_column(pool, PERSONAL_PROJECT_ID, binding)
            .await
            .unwrap()
            .unwrap();
        let card = cards::create_card(
            pool,
            cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: format!("Program {node_id}"),
                description: Some("program handoff fixture".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(column.id),
                created_by: Some("codex".to_string()),
                metadata_json: Some(serde_json::json!({
                    "depends_on": dependencies,
                })),
            },
        )
        .await
        .unwrap();
        let mut metadata = card.metadata_json.as_object().cloned().unwrap_or_default();
        metadata.insert(
            "program".to_string(),
            serde_json::json!({
                "program_id": &program.program_id,
                "node_id": node_id,
                "manifest_sha256": manifest_identity_hash(program).unwrap(),
            }),
        );
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::to_string(&metadata).unwrap())
            .bind(&card.id)
            .execute(pool)
            .await
            .unwrap();

        cards::get_card(pool, &card.id).await.unwrap().unwrap()
    }

    /// Stamp Council approval provenance on the mapped goals and land the
    /// answered `council_action` approval the bridge re-checks at consumption
    /// time (`require_current_approved_delivery`). Registration is not a
    /// permanent authorization, so a claim with no live approval fails closed —
    /// which is why a maintenance-tick fixture needs this and not just a
    /// manifest.
    async fn approve_council_program(
        pool: &Pool<Sqlite>,
        program: &ProgramDag,
        goals: &[(&cards::Card, &str)],
    ) {
        for (card, council_node_id) in goals {
            let current = cards::get_card(pool, &card.id).await.unwrap().unwrap();
            let mut metadata = current.metadata_json.as_object().cloned().unwrap();
            metadata.insert(
                "council_plan_id".to_string(),
                serde_json::json!("approved-council-program"),
            );
            metadata.insert(
                "council_node_id".to_string(),
                serde_json::json!(council_node_id),
            );
            sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
                .bind(serde_json::to_string(&metadata).unwrap())
                .bind(&card.id)
                .execute(pool)
                .await
                .unwrap();
        }
        let manifest = serde_yaml::to_string(program).unwrap();
        let manifest_hash = manifest_identity_hash(program).unwrap();
        let nodes: Vec<serde_json::Value> = program
            .nodes
            .iter()
            .map(|node| {
                serde_json::json!({
                    "id": node.id,
                    "title": format!("Run {}", node.id),
                    "description": format!("Complete {}", node.id),
                    "acceptance_criteria": [format!("{} complete", node.id)],
                    "dependencies": node.depends_on,
                    "estimated_budget": 1,
                    "risk": "low",
                    "verification": {"command": "true", "required": true}
                })
            })
            .collect();
        let decision = create_decision(
            pool,
            NewDecision {
                kind: "council_action".to_string(),
                project_id: Some(crate::projects::PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Approve Council program fixture".to_string()),
                detail: Some("fixture approval".to_string()),
                payload: serde_json::json!({
                    "session_id": "program-bridge-fixture",
                    "title": "Approve Council program fixture",
                    "plan": {
                        "proposal_id": "approved-council-program",
                        "program_manifest_sha256": manifest_hash,
                        "program_manifest": manifest,
                        "project": {
                            "project_id": crate::projects::PERSONAL_PROJECT_ID,
                            "project_name": "Personal",
                            "summary": "Program bridge fixture"
                        },
                        "budget_limit": 3,
                        "nodes": nodes
                    }
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        answer_decision(
            pool,
            &decision.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
    }

    async fn mark_complete(pool: &Pool<Sqlite>, card: &cards::Card) {
        let current = cards::get_card(pool, &card.id).await.unwrap().unwrap();
        let current_column = cards::get_column(pool, &current.column_id)
            .await
            .unwrap()
            .unwrap();
        if current_column.state_binding.as_deref() == Some("triage") {
            goal_transition::advance_goal_checked(
                pool,
                &card.id,
                GoalAction::Ready,
                decisions::ACTOR_SYSTEM,
                None,
                goal_transition::TransitionEffects::default(),
            )
            .await
            .unwrap();
        }
        let current = cards::get_card(pool, &card.id).await.unwrap().unwrap();
        let current_column = cards::get_column(pool, &current.column_id)
            .await
            .unwrap()
            .unwrap();
        if current_column.state_binding.as_deref() == Some("ready") {
            goal_transition::advance_goal_checked(
                pool,
                &card.id,
                GoalAction::Dispatch,
                decisions::ACTOR_SYSTEM,
                None,
                goal_transition::TransitionEffects::default(),
            )
            .await
            .unwrap();
        }
        let current = cards::get_card(pool, &card.id).await.unwrap().unwrap();
        let current_column = cards::get_column(pool, &current.column_id)
            .await
            .unwrap()
            .unwrap();
        if current_column.state_binding.as_deref() == Some("in_progress") {
            goal_transition::advance_goal_checked(
                pool,
                &card.id,
                GoalAction::Review,
                decisions::ACTOR_SYSTEM,
                None,
                goal_transition::TransitionEffects::default(),
            )
            .await
            .unwrap();
        }
        let current = cards::get_card(pool, &card.id).await.unwrap().unwrap();
        let current_column = cards::get_column(pool, &current.column_id)
            .await
            .unwrap()
            .unwrap();
        if current_column.state_binding.as_deref() == Some("review") {
            let decision = create_decision(
                pool,
                NewDecision {
                    kind: "approve_review".to_string(),
                    goal_id: Some(card.id.clone()),
                    project_id: Some(crate::projects::PERSONAL_PROJECT_ID.to_string()),
                    headline: Some("Approve bridge fixture completion".to_string()),
                    detail: Some("fixture evidence is complete".to_string()),
                    payload: serde_json::json!({}),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let (_, proof) = answer_decision(
                pool,
                &decision.id,
                &DecisionAnswer {
                    answer: "approve".to_string(),
                    ..Default::default()
                },
                decisions::ACTOR_JESSE,
            )
            .await
            .unwrap();
            goal_transition::advance_goal_checked(
                pool,
                &card.id,
                GoalAction::Approve,
                decisions::ACTOR_JESSE,
                Some(proof),
                goal_transition::TransitionEffects::default(),
            )
            .await
            .unwrap();
        }
        let mut receipt = execution_receipt::ExecutionReceipt::new(
            "fixture-worker",
            format!("session-{}", card.id),
            serde_json::json!({}),
            "fixture-lifecycle",
            "2026-09-05T00:00:00Z",
            1,
        );
        receipt.finalize(
            execution_receipt::ReceiptState::Completed,
            "2026-09-05T00:01:00Z",
        );
        cards::set_goal_execution_receipt(pool, &card.id, serde_json::to_value(receipt).unwrap())
            .await
            .unwrap();
        cards::set_goal_dispatch_evidence(
            pool,
            &card.id,
            serde_json::to_value(goal_engine::GoalEvidence::default()).unwrap(),
        )
        .await
        .unwrap();
        // This is the typed verifier result consumed by the handoff gate; the
        // dispatch evidence itself remains the production GoalEvidence shape.
        let current = cards::get_card(pool, &card.id).await.unwrap().unwrap();
        let mut metadata = current.metadata_json.as_object().cloned().unwrap();
        let mut evidence = metadata
            .get("dispatch_evidence")
            .and_then(|value| value.as_object())
            .cloned()
            .unwrap_or_default();
        evidence.insert(
            "verdict".to_string(),
            serde_json::json!({
                "status": "pass",
                "finished_at": "fixture-verification"
            }),
        );
        metadata.insert(
            "dispatch_evidence".to_string(),
            serde_json::Value::Object(evidence),
        );
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::to_string(&metadata).unwrap())
            .bind(&card.id)
            .execute(pool)
            .await
            .unwrap();

        // Keep the fixture honest: a handoff is allowed only after the real
        // lifecycle reaches Complete. This assertion must run after the
        // lifecycle and evidence writes, never while the card is constructed.
        let verified = cards::get_card(pool, &card.id).await.unwrap().unwrap();
        let verified_column = cards::get_column(pool, &verified.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            verified_column.state_binding.as_deref(),
            Some("complete"),
            "fixture goal {} did not reach terminal-success",
            card.id
        );
    }

    fn recorder(seen: Arc<Mutex<Vec<String>>>) -> orchestrator::GoalDispatchHook {
        Box::new(move |card_id| {
            seen.lock().unwrap().push(card_id);
            Box::pin(async { Ok("fixture-session".to_string()) })
        })
    }

    fn guarded_recorder(
        pool: Pool<Sqlite>,
        seen: Arc<Mutex<Vec<String>>>,
    ) -> orchestrator::GoalDispatchHook {
        Box::new(move |card_id| {
            let pool = pool.clone();
            let seen = Arc::clone(&seen);
            Box::pin(async move {
                goal_transition::advance_goal_checked(
                    &pool,
                    &card_id,
                    GoalAction::Dispatch,
                    decisions::ACTOR_SYSTEM,
                    None,
                    goal_transition::TransitionEffects::default(),
                )
                .await
                .map_err(|error| error.to_string())?;
                seen.lock().unwrap().push(card_id);
                Ok("fixture-session".to_string())
            })
        })
    }

    #[tokio::test]
    async fn database_handoff_runs_a_to_b_to_c_and_duplicate_is_idempotent() {
        let pool = bridge_pool().await;
        let program = chain_program();
        let a = mapped_goal(&pool, "a", "triage", &[], &program).await;
        let b = mapped_goal(&pool, "b", "triage", &[a.id.clone()], &program).await;
        let c = mapped_goal(&pool, "c", "triage", &[b.id.clone()], &program).await;
        mark_complete(&pool, &a).await;
        let manifest = serde_yaml::to_string(&program).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hook = recorder(Arc::clone(&seen));

        let first = ProgramHandoffRequest {
            source_goal_id: a.id.clone(),
            node_id: "a".to_string(),
            manifest: manifest.clone(),
            receipts: vec![ExitGateReceipt::passed("a done")],
        };
        let applied = apply_handoff_with_dispatch(&pool, first.clone(), Some(&hook))
            .await
            .unwrap();
        assert_eq!(applied.status, HandoffStatus::Applied);
        assert_eq!(applied.dispatched, 1);
        assert_eq!(seen.lock().unwrap().as_slice(), &[b.id.clone()]);

        let replay = apply_handoff_with_dispatch(&pool, first, Some(&hook))
            .await
            .unwrap();
        assert_eq!(replay.status, HandoffStatus::AlreadyApplied);
        assert_eq!(seen.lock().unwrap().len(), 1);

        // The fixture advances B to terminal-success so the next real bridge
        // handoff can exercise the same mapping and idempotency rail for C.
        mark_complete(&pool, &b).await;
        let next_program: ProgramDag = serde_yaml::from_str(&applied.manifest).unwrap();
        let next_manifest = serde_yaml::to_string(&next_program).unwrap();
        let second = apply_handoff_with_dispatch(
            &pool,
            ProgramHandoffRequest {
                source_goal_id: b.id.clone(),
                node_id: "b".to_string(),
                manifest: next_manifest,
                receipts: vec![ExitGateReceipt::passed("b done")],
            },
            Some(&hook),
        )
        .await
        .unwrap();
        assert_eq!(second.status, HandoffStatus::Applied);
        assert_eq!(second.dispatched, 1);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            &[b.id.clone(), c.id.clone()]
        );
        assert_eq!(next_program.nodes.len(), 3);
    }

    #[tokio::test]
    async fn maintenance_tick_retries_due_registered_claim_exactly_once() {
        let pool = bridge_pool().await;
        let program = chain_program();
        // Registered, not in-place: the maintenance tick has no caller to hand
        // it a manifest, so the claim is only replayable when registration
        // embedded one on the card.
        let a = mapped_goal(&pool, "a", "triage", &[], &program).await;
        let b = mapped_goal(&pool, "b", "triage", &[a.id.clone()], &program).await;
        let c = mapped_goal(&pool, "c", "triage", &[b.id.clone()], &program).await;
        // Go through the real approval + registration path. Hand-stamping the
        // card metadata reaches pending_dispatch but not a claim the
        // maintenance tick may settle: consumption re-checks live Council
        // provenance and the registered manifest, so a fixture that skips
        // registration fails closed exactly as production would.
        approve_council_program(&pool, &program, &[(&a, "a"), (&b, "b"), (&c, "c")]).await;
        register_program(
            &pool,
            ProgramRegistrationRequest {
                manifest: serde_yaml::to_string(&program).unwrap(),
                node_to_goal: BTreeMap::from([
                    ("a".to_string(), a.id.clone()),
                    ("b".to_string(), b.id.clone()),
                    ("c".to_string(), c.id.clone()),
                ]),
            },
        )
        .await
        .unwrap();
        mark_complete(&pool, &a).await;
        // Gate receipts are persisted by the verification pipeline, not handed
        // over by the completion caller, and they must match the current
        // verdict identity when one is present. The maintenance tick has no
        // caller to re-supply them, so a claim whose receipts were only ever in
        // the request is durable but unsettleable — persist them exactly the way
        // the completion seam does.
        let current = cards::get_card(&pool, &a.id).await.unwrap().unwrap();
        let mut metadata = current.metadata_json.as_object().cloned().unwrap();
        metadata.insert(
            "program_receipts".to_string(),
            serde_json::json!([{
                "gate": "a done",
                "passed": true,
                "verification_id": "fixture-verification"
            }]),
        );
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::to_string(&metadata).unwrap())
            .bind(&a.id)
            .execute(&pool)
            .await
            .unwrap();
        // Complete through the REGISTERED seam with no dispatcher installed, so
        // the durable claim carries the digest the registered path computes.
        // Going through the caller/CLI entry point instead leaves a claim whose
        // digest the maintenance retry cannot reproduce, and the retry then
        // fails closed with "already has a different transition digest" on
        // every tick — a durable claim nothing can ever settle.
        let pending = handoff_registered_goal_with_dispatch(&pool, &a.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pending.status, HandoffStatus::PendingDispatch);
        // Make the durable claim due. `program_transition` is a PROTECTED goal
        // metadata key (goal_transition::PROTECTED_GOAL_METADATA_KEYS): card
        // CRUD refuses to write it by design, and the bridge itself only ever
        // writes it inside its own validated transaction. So backdate the retry
        // clock through that same direct seam — going through the guarded
        // `update_card` is exactly what the guard exists to stop.
        let current = cards::get_card(&pool, &a.id).await.unwrap().unwrap();
        let mut metadata = current.metadata_json.as_object().cloned().unwrap();
        metadata["program_transition"]["next_attempt_at"] = serde_json::json!(0);
        let backdated = serde_json::to_string(&serde_json::Value::Object(metadata)).unwrap();
        let changed = sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(backdated)
            .bind(&a.id)
            .execute(&pool)
            .await
            .unwrap()
            .rows_affected();
        assert_eq!(changed, 1, "backdating the retry clock must hit the claim");

        let seen = Arc::new(Mutex::new(Vec::new()));
        let hook = guarded_recorder(pool.clone(), Arc::clone(&seen));
        let recovered =
            orchestrator::reconcile_pending_registered_dispatches(&pool, Some(&hook)).await;
        assert_eq!(recovered.auto_dispatched, 1);
        assert_eq!(recovered.approval_required, 0);
        assert_eq!(seen.lock().unwrap().as_slice(), &[b.id.clone()]);

        // The maintenance trigger sees no pending claim after the exact
        // transition is applied, so a replay cannot start B twice.
        let replay =
            orchestrator::reconcile_pending_registered_dispatches(&pool, Some(&hook)).await;
        assert_eq!(replay.examined, 0);
        assert_eq!(seen.lock().unwrap().as_slice(), &[b.id]);
    }

    #[tokio::test]
    async fn registered_program_advances_from_normal_completion_seam() {
        let pool = bridge_pool().await;
        let program = chain_program();
        let a = mapped_goal(&pool, "a", "triage", &[], &program).await;
        let b = mapped_goal(&pool, "b", "triage", &[a.id.clone()], &program).await;
        let c = mapped_goal(&pool, "c", "triage", &[b.id.clone()], &program).await;
        for (card, council_node_id) in [(&a, "a"), (&b, "b"), (&c, "c")] {
            let current = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
            let mut metadata = current.metadata_json.as_object().cloned().unwrap();
            metadata.insert(
                "council_plan_id".to_string(),
                serde_json::json!("approved-council-program"),
            );
            metadata.insert(
                "council_node_id".to_string(),
                serde_json::json!(council_node_id),
            );
            sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
                .bind(serde_json::to_string(&metadata).unwrap())
                .bind(&card.id)
                .execute(&pool)
                .await
                .unwrap();
        }
        let manifest = serde_yaml::to_string(&program).unwrap();
        let manifest_hash = manifest_identity_hash(&program).unwrap();
        let decision = create_decision(
            &pool,
            NewDecision {
                kind: "council_action".to_string(),
                project_id: Some(crate::projects::PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Approve Council program fixture".to_string()),
                detail: Some("fixture approval".to_string()),
                payload: serde_json::json!({
                    "session_id": "program-bridge-fixture",
                    "title": "Approve Council program fixture",
                    "plan": {
                        "proposal_id": "approved-council-program",
                        "program_manifest_sha256": manifest_hash,
                        "program_manifest": manifest.clone(),
                        "project": {
                            "project_id": crate::projects::PERSONAL_PROJECT_ID,
                            "project_name": "Personal",
                            "summary": "Program bridge fixture"
                        },
                        "budget_limit": 3,
                        "nodes": [
                            {
                                "id": "a",
                                "title": "Run A",
                                "description": "Complete A",
                                "acceptance_criteria": ["a complete"],
                                "estimated_budget": 1,
                                "risk": "low",
                                "verification": {"command": "true", "required": true}
                            },
                            {
                                "id": "b",
                                "title": "Run B",
                                "description": "Complete B",
                                "acceptance_criteria": ["b complete"],
                                "dependencies": ["a"],
                                "estimated_budget": 1,
                                "risk": "low",
                                "verification": {"command": "true", "required": true}
                            },
                            {
                                "id": "c",
                                "title": "Run C",
                                "description": "Complete C",
                                "acceptance_criteria": ["c complete"],
                                "dependencies": ["b"],
                                "estimated_budget": 1,
                                "risk": "low",
                                "verification": {"command": "true", "required": true}
                            }
                        ]
                    }
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        answer_decision(
            &pool,
            &decision.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        sqlx::query("UPDATE decisions SET payload_json = json_set(payload_json, '$.plan.program_manifest_sha256', 'tampered') WHERE id = ?")
            .bind(&decision.id)
            .execute(&pool)
            .await
            .unwrap();
        let request = ProgramRegistrationRequest {
            manifest: manifest.clone(),
            node_to_goal: BTreeMap::from([
                ("a".to_string(), a.id.clone()),
                ("b".to_string(), b.id.clone()),
                ("c".to_string(), c.id.clone()),
            ]),
        };
        let tampered = register_program(&pool, request.clone())
            .await
            .expect_err("approval hash mutation must block registration");
        assert!(
            matches!(tampered, ProgramHandoffError::Conflict(_)),
            "tampered approval hash must fail closed as a conflict, got {tampered:?}"
        );
        sqlx::query("UPDATE decisions SET payload_json = json_set(payload_json, '$.plan.program_manifest_sha256', ?) WHERE id = ?")
            .bind(&manifest_hash)
            .bind(&decision.id)
            .execute(&pool)
            .await
            .unwrap();
        let registration = register_program(&pool, request).await.unwrap();
        assert_eq!(registration.program_id, program.program_id);
        assert!(cards::get_card(&pool, &a.id)
            .await
            .unwrap()
            .unwrap()
            .metadata_json
            .get("program")
            .and_then(|value| value.get("manifest"))
            .is_some());

        mark_complete(&pool, &a).await;
        let current = cards::get_card(&pool, &a.id).await.unwrap().unwrap();
        let mut metadata = current.metadata_json.as_object().cloned().unwrap();
        metadata.insert(
            "program_receipts".to_string(),
            serde_json::json!([{
                "gate": "a done",
                "passed": true,
                "verification_id": "fixture-verification"
            }]),
        );
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::to_string(&metadata).unwrap())
            .bind(&a.id)
            .execute(&pool)
            .await
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hook = recorder(Arc::clone(&seen));
        // This is the same typed completion seam used by the approval effect;
        // no CLI manifest or worker-authored pass state is supplied.
        let response = handoff_registered_goal_with_dispatch(&pool, &a.id, Some(&hook))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status, HandoffStatus::Applied);
        assert_eq!(seen.lock().unwrap().as_slice(), &[b.id.clone()]);
        let a_after = cards::get_card(&pool, &a.id).await.unwrap().unwrap();
        assert_eq!(
            a_after
                .metadata_json
                .get("program_receipts")
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(1)
        );
        let b_after = cards::get_card(&pool, &b.id).await.unwrap().unwrap();
        let b_link: ProgramCardLink =
            serde_json::from_value(b_after.metadata_json.get("program").cloned().unwrap()).unwrap();
        let transitioned: ProgramDag = serde_yaml::from_str(&b_link.manifest.unwrap()).unwrap();
        assert_eq!(
            transitioned
                .nodes
                .iter()
                .find(|node| node.id == "b")
                .unwrap()
                .status,
            ProgramNodeStatus::Active
        );
    }

    #[test]
    fn registered_handoff_fails_closed_without_gate_specific_receipts() {
        let program = program(ProgramNodeStatus::Active);
        let node = program.nodes.first().unwrap();
        let error = load_registered_receipts(&serde_json::json!({}), node, "goal")
            .expect_err("generic completion evidence must not manufacture gate receipts");
        assert!(matches!(error, ProgramHandoffError::Pending(_)));

        let failed = load_registered_receipts(
            &serde_json::json!({
                "program_receipts": [{"gate": node.exit_gate[0], "passed": false}]
            }),
            node,
            "goal",
        )
        .expect_err("a failed gate receipt must not authorize continuation");
        assert!(matches!(failed, ProgramHandoffError::Pending(_)));
    }

    #[test]
    fn no_write_handoff_requires_trusted_empty_goal_evidence() {
        let evidence = serde_json::to_value(goal_engine::GoalEvidence::default()).unwrap();
        let mut metadata = serde_json::json!({"dispatch_evidence": evidence});
        require_no_write_evidence(&metadata, "goal").unwrap();

        metadata["dispatch_evidence"]["files_changed"] = serde_json::json!(1);
        assert!(matches!(
            require_no_write_evidence(&metadata, "goal"),
            Err(ProgramHandoffError::Invalid(_))
        ));

        metadata["dispatch_evidence"]["files_changed"] = serde_json::json!(0);
        metadata["dispatch_evidence"]["diff_errored"] = serde_json::json!(true);
        assert!(matches!(
            require_no_write_evidence(&metadata, "goal"),
            Err(ProgramHandoffError::Invalid(_))
        ));
    }

    #[test]
    fn registered_receipts_must_match_current_verification_identity() {
        let program = program(ProgramNodeStatus::Active);
        let node = program.nodes.first().unwrap();
        let metadata = serde_json::json!({
            "dispatch_evidence": {"verdict": {"status": "pass", "finished_at": "new-run"}},
            "program_receipts": [{
                "gate": node.exit_gate[0],
                "passed": true,
                "verification_id": "old-run"
            }]
        });
        assert!(matches!(
            load_registered_receipts(&metadata, node, "goal"),
            Err(ProgramHandoffError::Pending(_))
        ));
    }

    #[tokio::test]
    async fn in_place_transition_manifest_retries_existing_pending_claim() {
        let pool = bridge_pool().await;
        let program = chain_program();
        let a = mapped_goal(&pool, "a", "triage", &[], &program).await;
        let b = mapped_goal(&pool, "b", "triage", &[a.id.clone()], &program).await;
        let _c = mapped_goal(&pool, "c", "triage", &[b.id.clone()], &program).await;
        mark_complete(&pool, &a).await;
        let request = ProgramHandoffRequest {
            source_goal_id: a.id.clone(),
            node_id: "a".to_string(),
            manifest: serde_yaml::to_string(&program).unwrap(),
            receipts: vec![ExitGateReceipt::passed("a done")],
        };
        let pending = apply_handoff_with_dispatch(&pool, request.clone(), None)
            .await
            .unwrap();
        assert_eq!(pending.status, HandoffStatus::PendingDispatch);

        // This is the exact manifest written by the CLI's --in-place path.
        // The source node is now Passed, so retry must use the durable claim
        // rather than pretending the Active -> Passed transition is new.
        let hook = recorder(Arc::new(Mutex::new(Vec::new())));
        let retry = apply_handoff_with_dispatch(
            &pool,
            ProgramHandoffRequest {
                manifest: pending.manifest,
                ..request
            },
            Some(&hook),
        )
        .await
        .unwrap();
        assert_eq!(retry.status, HandoffStatus::Applied);
        assert_eq!(retry.dispatched, 1);
    }

    #[tokio::test]
    async fn mixed_activation_never_dispatches_approval_required_and_retries_pending() {
        let pool = bridge_pool().await;
        let mut program = chain_program();
        program.program_id = "mixed-handoff".to_string();
        program.nodes[0].next_on_pass = vec!["b".to_string(), "c".to_string()];
        program.nodes[2].depends_on = vec!["a".to_string(), "b".to_string()];
        program.nodes[2].approval = ApprovalPolicy::Human;
        let a = mapped_goal(&pool, "a", "triage", &[], &program).await;
        let b = mapped_goal(&pool, "b", "triage", &[a.id.clone()], &program).await;
        let c = mapped_goal(
            &pool,
            "c",
            "triage",
            &[a.id.clone(), b.id.clone()],
            &program,
        )
        .await;
        mark_complete(&pool, &a).await;
        let request = ProgramHandoffRequest {
            source_goal_id: a.id,
            node_id: "a".to_string(),
            manifest: serde_yaml::to_string(&program).unwrap(),
            receipts: vec![ExitGateReceipt::passed("a done")],
        };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hook = recorder(Arc::clone(&seen));
        let pending = apply_handoff_with_dispatch(&pool, request.clone(), None)
            .await
            .unwrap();
        assert_eq!(pending.status, HandoffStatus::PendingDispatch);
        assert!(seen.lock().unwrap().is_empty());
        let b_after_pending = cards::get_card(&pool, &b.id).await.unwrap().unwrap();
        let b_pending_column = cards::get_column(&pool, &b_after_pending.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            b_pending_column.state_binding.as_deref(),
            Some("ready"),
            "approval-free successor is Ready but not dispatched while the hook is absent"
        );
        let c_after_pending = cards::get_card(&pool, &c.id).await.unwrap().unwrap();
        let c_pending_column = cards::get_column(&pool, &c_after_pending.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            c_pending_column.state_binding.as_deref(),
            Some("triage"),
            "approval-gated successor remains Triage until its dependencies pass"
        );
        let retried = apply_handoff_with_dispatch(&pool, request, Some(&hook))
            .await
            .unwrap();
        assert_eq!(retried.status, HandoffStatus::Applied);
        assert!(retried.approval_required.is_empty());
        assert_eq!(seen.lock().unwrap().as_slice(), &[b.id.clone()]);
        assert!(!seen.lock().unwrap().contains(&c.id));

        // C depends on B as well as A, so the approval gate becomes eligible
        // only after B has itself passed through the same handoff rail.
        mark_complete(&pool, &b).await;
        let second = apply_handoff_with_dispatch(
            &pool,
            ProgramHandoffRequest {
                source_goal_id: b.id.clone(),
                node_id: "b".to_string(),
                manifest: retried.manifest,
                receipts: vec![ExitGateReceipt::passed("b done")],
            },
            Some(&hook),
        )
        .await
        .unwrap();
        assert_eq!(second.status, HandoffStatus::ApprovalRequired);
        assert_eq!(second.approval_required, vec!["c".to_string()]);
        assert_eq!(seen.lock().unwrap().as_slice(), &[b.id]);
        let c_after_b = cards::get_card(&pool, &c.id).await.unwrap().unwrap();
        let c_after_b_column = cards::get_column(&pool, &c_after_b.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            c_after_b_column.state_binding.as_deref(),
            Some("triage"),
            "approval-gated successor is not dispatched before explicit approval"
        );
    }

    #[tokio::test]
    async fn paused_project_leaves_successor_in_triage_until_retry() {
        use crate::projects::{add_tag, remove_tag, PERSONAL_PROJECT_ID};
        let pool = bridge_pool().await;
        let program = chain_program();
        let a = mapped_goal(&pool, "a", "triage", &[], &program).await;
        let b = mapped_goal(&pool, "b", "triage", &[a.id.clone()], &program).await;
        let _c = mapped_goal(&pool, "c", "triage", &[b.id.clone()], &program).await;
        mark_complete(&pool, &a).await;
        add_tag(&pool, PERSONAL_PROJECT_ID, "roadmap_paused")
            .await
            .unwrap();
        let request = ProgramHandoffRequest {
            source_goal_id: a.id,
            node_id: "a".to_string(),
            manifest: serde_yaml::to_string(&program).unwrap(),
            receipts: vec![ExitGateReceipt::passed("a done")],
        };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hook = recorder(Arc::clone(&seen));
        let paused = apply_handoff_with_dispatch(&pool, request.clone(), Some(&hook))
            .await
            .unwrap();
        assert_eq!(paused.status, HandoffStatus::PendingDispatch);
        assert!(seen.lock().unwrap().is_empty());
        let triage = cards::get_card(&pool, &b.id).await.unwrap().unwrap();
        let triage_column = cards::get_column(&pool, &triage.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(triage_column.state_binding.as_deref(), Some("triage"));
        remove_tag(&pool, PERSONAL_PROJECT_ID, "roadmap_paused")
            .await
            .unwrap();
        let resumed = apply_handoff_with_dispatch(&pool, request, Some(&hook))
            .await
            .unwrap();
        assert_eq!(resumed.status, HandoffStatus::Applied);
        assert_eq!(seen.lock().unwrap().as_slice(), &[b.id]);
    }

    #[tokio::test]
    async fn concurrent_delivery_has_one_guarded_dispatch() {
        let pool = bridge_pool().await;
        let program = chain_program();
        let a = mapped_goal(&pool, "a", "triage", &[], &program).await;
        let _b = mapped_goal(&pool, "b", "triage", &[a.id.clone()], &program).await;
        let _c = mapped_goal(&pool, "c", "triage", &[_b.id.clone()], &program).await;
        mark_complete(&pool, &a).await;
        let request = ProgramHandoffRequest {
            source_goal_id: a.id,
            node_id: "a".to_string(),
            manifest: serde_yaml::to_string(&program).unwrap(),
            receipts: vec![ExitGateReceipt::passed("a done")],
        };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hook = guarded_recorder(pool.clone(), Arc::clone(&seen));
        let (left, right) = tokio::join!(
            apply_handoff_with_dispatch(&pool, request.clone(), Some(&hook)),
            apply_handoff_with_dispatch(&pool, request, Some(&hook)),
        );
        assert!(left.is_ok(), "first concurrent handoff failed: {left:?}");
        assert!(right.is_ok(), "second concurrent handoff failed: {right:?}");
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
