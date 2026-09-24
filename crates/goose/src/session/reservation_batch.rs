//! K1b: durable Council convene batch admission (Astra's ruled Option D).
//!
//! A multi-provider convene must authorise its whole fan-out ONCE, before any
//! seat is called. There is NO monetary parent reservation: admission
//! atomically writes one ORDINARY `pending` row in `cost_reservations` per
//! paid slot, plus a manifest (`cost_reservation_batches` +
//! `cost_reservation_batch_members`) binding the frozen plan to those rows.
//! Liability therefore stays `settled + pending + unknown` and every planned
//! invocation is counted exactly once — there is nothing to double-count.
//!
//! Invariants enforced here (see the K1b notes for the full map):
//! - admission is ONE `BEGIN IMMEDIATE`: expired-batch reconciliation, the
//!   idempotency check, the gate-approval check and consumption, the ceiling
//!   arithmetic on the SUM of paid bounds, and every insert commit together or
//!   not at all;
//! - the ceiling arithmetic is the single-invocation path's own
//!   (`reservation_ceiling_verdict`), not a copy;
//! - a claim transfers the EXISTING reservation to the child session and never
//!   creates one;
//! - close/expiry release ONLY unclaimed slots (claim-before-wire proves they
//!   never dispatched) and retain every claimed one under U1's rules.
use super::session_manager::{
    reservation_ceiling_verdict, reservation_ceilings, CostReservationOutcome, SessionStorage,
};
use super::{budget_task_id, ExtensionData};
use crate::cost_router::budget::{BudgetConfig, BudgetScope};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite, Transaction};

/// How long past the batch's claim deadline each paid slot's reservation
/// lease runs. A slot claimed just before the deadline still has this long to
/// settle before the ordinary lease sweep turns it `unknown`.
pub const BATCH_CLAIM_SETTLE_GRACE_SECS: i64 = 3600;

/// The Council card's approving option id (see `council/preparation.rs`).
/// Choice decisions store `answer = 'choice'` plus `answer_choice_id`; an
/// approval is never recognised by `answer = 'approve'`.
pub const COUNCIL_APPROVE_CHOICE_ID: &str = "approve";

/// The durable proposal marker the Council card carries:
/// `council_convene:{request_id}:{plan_hash}`.
pub fn council_proposal_marker(request_id: &str, plan_hash: &str) -> String {
    format!("council_convene:{request_id}:{plan_hash}")
}

/// One planned logical invocation with its retry allowance — not a whole seat.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchSlot {
    pub slot_id: String,
    pub invocation_id: String,
    pub provider: String,
    pub model: Option<String>,
    /// `None` is a free slot: recorded in the manifest, never reserved.
    pub bound_usd: Option<f64>,
}

/// A REFERENCE to a human approval, never proof of one. Admission loads and
/// verifies the decision itself.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchGateApproval {
    pub decision_id: String,
    pub plan_hash: String,
}

#[derive(Debug, Clone)]
pub struct BatchAdmissionRequest {
    pub request_id: String,
    pub plan_hash: String,
    pub owner_session_id: String,
    pub slots: Vec<BatchSlot>,
    /// RFC3339 deadline for CLAIMING slots.
    pub lease_until: String,
    pub gate_approval: Option<BatchGateApproval>,
}

/// Mirrors `CostReservationOutcome` so NeedsGate/Refused carry the same
/// evidence fields and an Inbox card can state the real number.
#[derive(Debug, Clone, PartialEq)]
pub enum BatchAdmissionOutcome {
    Admitted {
        batch_id: String,
        reserved_usd: f64,
        paid_slots: usize,
    },
    AlreadyAdmitted {
        batch_id: String,
    },
    PlanChanged {
        batch_id: String,
        admitted_plan_hash: String,
    },
    NeedsGate {
        scope: BudgetScope,
        spent_usd: f64,
        held_usd: f64,
        requested_usd: f64,
        ceiling_usd: f64,
    },
    Refused {
        scope: BudgetScope,
        spent_usd: f64,
        held_usd: f64,
        requested_usd: f64,
        ceiling_usd: f64,
    },
    Unknown {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum BatchClaimOutcome {
    /// The slot's EXISTING reservation, attribution moved to the child.
    Claimed {
        reservation_id: String,
        bound_usd: f64,
    },
    AlreadyClaimed {
        reservation_id: String,
        claimed_session_id: String,
    },
    BatchClosed,
    BatchExpired,
    NotEligible {
        reason: String,
    },
    UnknownSlot,
}

/// `released_*` counts only slots NEWLY released by this call (zero on a
/// repeat). `retained_slots` are claimed slots whose reservation is still
/// funded (`pending`/`unknown`) — never refunded here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BatchCloseSummary {
    pub released_slots: usize,
    pub released_usd: f64,
    pub retained_slots: usize,
}

/// What one expiry reconciliation pass did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BatchExpirySweep {
    pub expired_batches: usize,
    pub released_slots: usize,
    pub released_usd: f64,
    pub retained_slots: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum BatchAdmissionError {
    /// The approval already admitted a batch. A human approval admits once.
    #[error("council_approval_consumed: approval {decision_id} already admitted batch {batch_id}")]
    ApprovalConsumed {
        decision_id: String,
        batch_id: String,
    },
    /// The referenced decision does not authorise THIS frozen batch.
    #[error("council_approval_invalid: approval {decision_id} rejected: {reason}")]
    ApprovalInvalid { decision_id: String, reason: String },
    /// The decision was not answered by the authenticated operator.
    #[error("council_approval_unauthorized: approval {decision_id} rejected: {reason}")]
    ApprovalUnauthorized { decision_id: String, reason: String },
    #[error("council_batch_unknown: no reservation batch {0}")]
    UnknownBatch(String),
    #[error("council_batch_storage: {0}")]
    Storage(#[from] anyhow::Error),
}

impl From<sqlx::Error> for BatchAdmissionError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(error.into())
    }
}

fn parse_lease(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|lease| lease.with_timezone(&Utc))
}

fn same_amount(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// Server-side digest of the submitted slots. The Council `plan_hash` covers
/// fields the batch request does not carry (members, chair, attempts), so it
/// cannot be recomputed here; this digest is what makes a replay with the
/// same claimed hash but different slots a `PlanChanged`, not an
/// `AlreadyAdmitted`.
fn slots_digest(slots: &[BatchSlot]) -> String {
    let mut sorted: Vec<&BatchSlot> = slots.iter().collect();
    sorted.sort_by(|a, b| a.slot_id.cmp(&b.slot_id));
    let canonical = serde_json::Value::Array(
        sorted
            .iter()
            .map(|s| {
                serde_json::json!([s.slot_id, s.invocation_id, s.provider, s.model, s.bound_usd])
            })
            .collect(),
    );
    hex::encode(Sha256::digest(canonical.to_string().as_bytes()))
}

/// Shape checks that need no storage. Returns the recomputed SUM of paid
/// bounds and the paid-slot count; never trusts a caller-supplied total.
fn validate_request(request: &BatchAdmissionRequest) -> Result<(f64, usize), String> {
    for (label, value) in [
        ("request_id", &request.request_id),
        ("plan_hash", &request.plan_hash),
        ("owner_session_id", &request.owner_session_id),
    ] {
        if value.trim().is_empty() {
            return Err(format!("batch {label} must not be empty"));
        }
    }
    if request.slots.is_empty() {
        return Err("batch must plan at least one slot".to_string());
    }
    let mut slot_ids = std::collections::HashSet::new();
    let mut invocation_ids = std::collections::HashSet::new();
    let mut sum = 0.0_f64;
    let mut paid = 0usize;
    for slot in &request.slots {
        if slot.slot_id.trim().is_empty()
            || slot.invocation_id.trim().is_empty()
            || slot.provider.trim().is_empty()
        {
            return Err("every batch slot needs a slot_id, invocation_id and provider".into());
        }
        if !slot_ids.insert(slot.slot_id.as_str()) {
            return Err(format!("duplicate batch slot_id {}", slot.slot_id));
        }
        if !invocation_ids.insert(slot.invocation_id.as_str()) {
            return Err(format!(
                "duplicate batch invocation_id {}",
                slot.invocation_id
            ));
        }
        if let Some(bound) = slot.bound_usd {
            if !bound.is_finite() || bound <= 0.0 {
                return Err(format!(
                    "paid slot {} must have a finite positive bound",
                    slot.slot_id
                ));
            }
            sum += bound;
            paid += 1;
        }
    }
    if !sum.is_finite() {
        return Err("batch bound is not finite".to_string());
    }
    Ok((sum, paid))
}

fn batch_outcome(outcome: CostReservationOutcome) -> BatchAdmissionOutcome {
    match outcome {
        CostReservationOutcome::NeedsGate {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        } => BatchAdmissionOutcome::NeedsGate {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        },
        CostReservationOutcome::Refused {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        } => BatchAdmissionOutcome::Refused {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        },
        CostReservationOutcome::Unknown { reason } => BatchAdmissionOutcome::Unknown { reason },
        other => BatchAdmissionOutcome::Unknown {
            reason: format!("unexpected ceiling verdict {other:?}"),
        },
    }
}

/// Verify a gate approval REFERENCE inside the admission transaction.
///
/// It must: not already be consumed; name this plan; be an answered Council
/// `choice` whose chosen option is the bounded approval; carry this request's
/// proposal marker (request_id + plan_hash) and owner session; state the same
/// total bound we recomputed from the submitted slots; and be answered by the
/// operator (`ACTOR_JESSE`) with an audit row naming a non-empty authenticated
/// principal. `ACTOR_HEADLESS_POLICY` is rejected HERE ONLY — it keeps working
/// for `auto_approve_reservation_gate` elsewhere.
async fn verify_gate_approval(
    tx: &mut Transaction<'_, Sqlite>,
    approval: &BatchGateApproval,
    request: &BatchAdmissionRequest,
    recomputed_bound: f64,
) -> Result<(), BatchAdmissionError> {
    let decision_id = approval.decision_id.clone();
    let invalid = |reason: String| BatchAdmissionError::ApprovalInvalid {
        decision_id: decision_id.clone(),
        reason,
    };
    let unauthorized = |reason: String| BatchAdmissionError::ApprovalUnauthorized {
        decision_id: decision_id.clone(),
        reason,
    };

    if let Some(batch_id) = sqlx::query_scalar::<_, String>(
        "SELECT batch_id FROM cost_reservation_batches WHERE approval_decision_id = ?",
    )
    .bind(&approval.decision_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Err(BatchAdmissionError::ApprovalConsumed {
            decision_id: decision_id.clone(),
            batch_id,
        });
    }
    if approval.plan_hash != request.plan_hash {
        return Err(invalid("approval names a different plan_hash".into()));
    }

    type DecisionRow = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    );
    let Some((kind, status, answer, choice_id, acted_by, payload_json)) =
        sqlx::query_as::<_, DecisionRow>(
            "SELECT kind, status, answer, answer_choice_id, acted_by, payload_json
             FROM decisions WHERE id = ?",
        )
        .bind(&approval.decision_id)
        .fetch_optional(&mut **tx)
        .await?
    else {
        return Err(invalid("decision does not exist".into()));
    };
    if kind != "choice" {
        return Err(invalid(format!("decision kind is '{kind}', not 'choice'")));
    }
    if status != "answered" {
        return Err(invalid(format!(
            "decision status is '{status}', not answered"
        )));
    }
    if answer.as_deref() != Some("choice")
        || choice_id.as_deref() != Some(COUNCIL_APPROVE_CHOICE_ID)
    {
        return Err(invalid(
            "decision did not choose the bounded Council approval".into(),
        ));
    }
    let payload: serde_json::Value = serde_json::from_str(&payload_json)
        .map_err(|e| invalid(format!("decision payload is unreadable: {e}")))?;
    let expected = council_proposal_marker(&request.request_id, &request.plan_hash);
    if payload["proposal"].as_str() != Some(expected.as_str()) {
        return Err(invalid(
            "decision does not authorise this Council request and plan".into(),
        ));
    }
    let gate = &payload["reservation_gate"];
    if !gate.is_object() {
        return Err(invalid(
            "decision carries no reservation-gate evidence".into(),
        ));
    }
    if gate["requesting_session_id"].as_str() != Some(request.owner_session_id.as_str()) {
        return Err(invalid(
            "decision was raised for a different owner session".into(),
        ));
    }
    let Some(approved_bound) = gate["requested_usd"].as_f64() else {
        return Err(invalid("decision states no approved bound".into()));
    };
    if !same_amount(approved_bound, recomputed_bound) {
        return Err(invalid(format!(
            "approved bound ${approved_bound:.6} does not match the submitted plan's bound ${recomputed_bound:.6}"
        )));
    }

    match acted_by.as_deref() {
        Some(actor) if actor == crate::decisions::ACTOR_JESSE => {}
        Some(actor) if actor == crate::decisions::ACTOR_HEADLESS_POLICY => {
            return Err(unauthorized(
                "a headless policy may not lift a Council budget gate".into(),
            ));
        }
        other => {
            return Err(unauthorized(format!(
                "decision was answered by {other:?}, not the operator"
            )));
        }
    }
    let audited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM decision_audit
         WHERE decision_id = ? AND acted_by = ? AND outcome = 'choice'
           AND principal IS NOT NULL AND length(trim(principal)) > 0",
    )
    .bind(&approval.decision_id)
    .bind(crate::decisions::ACTOR_JESSE)
    .fetch_one(&mut **tx)
    .await?;
    if audited == 0 {
        return Err(unauthorized(
            "the operator's answer has no audited authenticated principal".into(),
        ));
    }
    Ok(())
}

/// Release every unclaimed paid slot of `batch_id` and move an OPEN batch to
/// `target_status`. Claimed slots are retained untouched (U1 rules decide
/// their fate). `None` when the batch does not exist.
///
/// An unclaimed slot's reservation is released from `pending` OR `unknown`:
/// claim-before-wire proves it never dispatched, so an `unknown` there can
/// only have come from the lease sweep, not from an ambiguous send.
async fn release_batch_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    batch_id: &str,
    target_status: &str,
    now: &str,
) -> Result<Option<BatchCloseSummary>, BatchAdmissionError> {
    let Some(status) = sqlx::query_scalar::<_, String>(
        "SELECT status FROM cost_reservation_batches WHERE batch_id = ?",
    )
    .bind(batch_id)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };
    if status == "open" {
        sqlx::query(
            "UPDATE cost_reservation_batches SET status = ?, updated_at = ?
             WHERE batch_id = ? AND status = 'open'",
        )
        .bind(target_status)
        .bind(now)
        .bind(batch_id)
        .execute(&mut **tx)
        .await?;
    }
    let members: Vec<(String, f64, Option<String>)> = sqlx::query_as(
        "SELECT reservation_id, bound_usd, claimed_session_id
         FROM cost_reservation_batch_members
         WHERE batch_id = ? AND reservation_id IS NOT NULL",
    )
    .bind(batch_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut summary = BatchCloseSummary::default();
    for (reservation_id, bound_usd, claimed) in members {
        if claimed.is_some() {
            let funded: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM cost_reservations
                 WHERE reservation_id = ? AND state IN ('pending', 'unknown')",
            )
            .bind(&reservation_id)
            .fetch_one(&mut **tx)
            .await?;
            if funded > 0 {
                summary.retained_slots += 1;
            }
            continue;
        }
        let released = sqlx::query(
            "UPDATE cost_reservations SET state = 'released', updated_at = ?
             WHERE reservation_id = ? AND state IN ('pending', 'unknown')",
        )
        .bind(now)
        .bind(&reservation_id)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if released == 1 {
            summary.released_slots += 1;
            summary.released_usd += bound_usd;
        }
    }
    Ok(Some(summary))
}

/// Expire every OPEN batch whose claim deadline has passed (an unparsable
/// deadline counts as passed — fail closed), inside the caller's transaction.
async fn reconcile_expired_batches_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    now: DateTime<Utc>,
) -> Result<BatchExpirySweep, BatchAdmissionError> {
    let now_s = now.to_rfc3339();
    let open: Vec<(String, String)> = sqlx::query_as(
        "SELECT batch_id, lease_until FROM cost_reservation_batches WHERE status = 'open'",
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut sweep = BatchExpirySweep::default();
    for (batch_id, lease) in open {
        if parse_lease(&lease).is_some_and(|lease| lease > now) {
            continue;
        }
        if let Some(summary) = release_batch_in_tx(tx, &batch_id, "expired", &now_s).await? {
            sweep.expired_batches += 1;
            sweep.released_slots += summary.released_slots;
            sweep.released_usd += summary.released_usd;
            sweep.retained_slots += summary.retained_slots;
        }
    }
    Ok(sweep)
}

/// Admit a frozen Council plan as one atomic batch. See the module docs.
pub(crate) async fn reserve_invocation_batch(
    pool: &Pool<Sqlite>,
    request: &BatchAdmissionRequest,
    config: &BudgetConfig,
) -> Result<BatchAdmissionOutcome, BatchAdmissionError> {
    let (paid_sum, paid_slots) = match validate_request(request) {
        Ok(totals) => totals,
        Err(reason) => return Ok(BatchAdmissionOutcome::Unknown { reason }),
    };
    let now = Utc::now();
    let Some(lease) = parse_lease(&request.lease_until) else {
        return Ok(BatchAdmissionOutcome::Unknown {
            reason: "batch lease must be RFC3339".to_string(),
        });
    };
    if lease <= now {
        return Ok(BatchAdmissionOutcome::Unknown {
            reason: "batch lease is already expired".to_string(),
        });
    }
    let Some((task_ceilings, session_ceilings)) = reservation_ceilings(config) else {
        return Ok(BatchAdmissionOutcome::Unknown {
            reason: "budget ceiling is unavailable or invalid".to_string(),
        });
    };
    let digest = slots_digest(&request.slots);
    let now_s = now.to_rfc3339();

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

    // Correctness must not depend on a timer: lapsed batches release their
    // unclaimed slots before this admission reads any held total.
    reconcile_expired_batches_in_tx(&mut tx, now).await?;

    // Idempotent on request_id. The same frozen plan is a replay; anything
    // else needs fresh authorization, never reuse of an old admission.
    if let Some((batch_id, admitted_plan_hash, admitted_digest)) =
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT batch_id, plan_hash, slots_digest FROM cost_reservation_batches
             WHERE request_id = ?",
        )
        .bind(&request.request_id)
        .fetch_optional(&mut *tx)
        .await?
    {
        tx.commit().await?;
        return Ok(
            if admitted_plan_hash == request.plan_hash && admitted_digest == digest {
                BatchAdmissionOutcome::AlreadyAdmitted { batch_id }
            } else {
                BatchAdmissionOutcome::PlanChanged {
                    batch_id,
                    admitted_plan_hash,
                }
            },
        );
    }

    let owner_extension: Option<String> =
        sqlx::query_scalar("SELECT extension_data FROM sessions WHERE id = ?")
            .bind(&request.owner_session_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(owner_extension) = owner_extension else {
        tx.commit().await?;
        return Ok(BatchAdmissionOutcome::Unknown {
            reason: format!(
                "batch owner session '{}' does not exist",
                request.owner_session_id
            ),
        });
    };
    let Some(task_id) = serde_json::from_str::<ExtensionData>(&owner_extension)
        .ok()
        .and_then(|data| budget_task_id(&data))
    else {
        tx.commit().await?;
        return Ok(BatchAdmissionOutcome::Unknown {
            reason: "batch owner session has no durable budget task identity".to_string(),
        });
    };

    for slot in &request.slots {
        let taken: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM cost_reservations WHERE invocation_id = ?)
                  + (SELECT COUNT(*) FROM cost_reservation_batch_members WHERE invocation_id = ?)",
        )
        .bind(&slot.invocation_id)
        .bind(&slot.invocation_id)
        .fetch_one(&mut *tx)
        .await?;
        if taken > 0 {
            tx.commit().await?;
            return Ok(BatchAdmissionOutcome::Unknown {
                reason: format!(
                    "planned invocation {} is already bound to another reservation",
                    slot.invocation_id
                ),
            });
        }
    }

    // A reference, not proof: verified (and later consumed) in THIS
    // transaction. Any failure returns before commit, so the approval stays
    // unconsumed and the reconciliation above simply reruns next time.
    let approved = match &request.gate_approval {
        Some(approval) => {
            verify_gate_approval(&mut tx, approval, request, paid_sum).await?;
            true
        }
        None => false,
    };

    if paid_sum > 0.0 {
        // U1's pre-steps, exactly as the single-invocation path runs them.
        SessionStorage::expire_lapsed_pending_reservations(&mut tx, &now_s).await?;
        SessionStorage::reconcile_stale_reservations(
            &mut tx,
            &task_id,
            &request.owner_session_id,
            &now_s,
        )
        .await?;
        match reservation_ceiling_verdict(
            &mut tx,
            &task_id,
            &request.owner_session_id,
            paid_sum,
            task_ceilings,
            session_ceilings,
        )
        .await?
        {
            None => {}
            // The approval lifts the SOFT gate for this one frozen batch only.
            // A hard ceiling is `Refused` above it, approval or not.
            Some(CostReservationOutcome::NeedsGate { .. }) if approved => {}
            Some(outcome) => {
                tx.commit().await?;
                return Ok(batch_outcome(outcome));
            }
        }
    }

    let batch_id = format!("batch:{}", request.request_id);
    sqlx::query(
        "INSERT INTO cost_reservation_batches
            (batch_id, request_id, plan_hash, slots_digest, task_id, owner_session_id,
             status, lease_until, bound_usd, approval_decision_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'open', ?, ?, ?, ?, ?)",
    )
    .bind(&batch_id)
    .bind(&request.request_id)
    .bind(&request.plan_hash)
    .bind(&digest)
    .bind(&task_id)
    .bind(&request.owner_session_id)
    .bind(lease.to_rfc3339())
    .bind(paid_sum)
    .bind(
        request
            .gate_approval
            .as_ref()
            .map(|approval| approval.decision_id.clone()),
    )
    .bind(&now_s)
    .bind(&now_s)
    .execute(&mut *tx)
    .await?;

    let reservation_lease =
        (lease + chrono::Duration::seconds(BATCH_CLAIM_SETTLE_GRACE_SECS)).to_rfc3339();
    for slot in &request.slots {
        let reservation_id = slot
            .bound_usd
            .map(|_| format!("invocation:{}", slot.invocation_id));
        if let (Some(reservation_id), Some(bound_usd)) = (&reservation_id, slot.bound_usd) {
            // An ORDINARY pending reservation, same id scheme and columns as
            // `reserve_provider_invocation`. No new state, no parent row.
            sqlx::query(
                "INSERT INTO cost_reservations
                    (reservation_id, invocation_id, session_id, task_id, amount_usd,
                     state, lease_until, created_at, updated_at, extension_json)
                 VALUES (?, ?, ?, ?, ?, 'pending', ?, ?, ?, NULL)",
            )
            .bind(reservation_id)
            .bind(&slot.invocation_id)
            .bind(&request.owner_session_id)
            .bind(&task_id)
            .bind(bound_usd)
            .bind(&reservation_lease)
            .bind(&now_s)
            .bind(&now_s)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO cost_reservation_batch_members
                (batch_id, slot_id, reservation_id, invocation_id, provider, model, bound_usd)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&batch_id)
        .bind(&slot.slot_id)
        .bind(&reservation_id)
        .bind(&slot.invocation_id)
        .bind(&slot.provider)
        .bind(&slot.model)
        .bind(slot.bound_usd)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(BatchAdmissionOutcome::Admitted {
        batch_id,
        reserved_usd: paid_sum,
        paid_slots,
    })
}

/// Claim one admitted slot for the child session about to dispatch it.
/// Transfers the EXISTING reservation's attribution to the child and returns
/// its id; never creates a reservation.
pub(crate) async fn claim_batch_invocation(
    pool: &Pool<Sqlite>,
    batch_id: &str,
    slot_id: &str,
    child_session_id: &str,
) -> Result<BatchClaimOutcome, BatchAdmissionError> {
    let now = Utc::now();
    let now_s = now.to_rfc3339();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let Some((status, lease, task_id, owner)) =
        sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT status, lease_until, task_id, owner_session_id
             FROM cost_reservation_batches WHERE batch_id = ?",
        )
        .bind(batch_id)
        .fetch_optional(&mut *tx)
        .await?
    else {
        tx.commit().await?;
        return Ok(BatchClaimOutcome::UnknownSlot);
    };
    match status.as_str() {
        "expired" => {
            tx.commit().await?;
            return Ok(BatchClaimOutcome::BatchExpired);
        }
        "closed" => {
            tx.commit().await?;
            return Ok(BatchClaimOutcome::BatchClosed);
        }
        _ => {}
    }
    if !parse_lease(&lease).is_some_and(|lease| lease > now) {
        // The deadline passed without a sweep: expire it now, releasing the
        // unclaimed slots under the same retention rules.
        release_batch_in_tx(&mut tx, batch_id, "expired", &now_s).await?;
        tx.commit().await?;
        return Ok(BatchClaimOutcome::BatchExpired);
    }
    let Some((reservation_id, bound_usd, claimed_session_id)) =
        sqlx::query_as::<_, (Option<String>, Option<f64>, Option<String>)>(
            "SELECT reservation_id, bound_usd, claimed_session_id
             FROM cost_reservation_batch_members WHERE batch_id = ? AND slot_id = ?",
        )
        .bind(batch_id)
        .bind(slot_id)
        .fetch_optional(&mut *tx)
        .await?
    else {
        tx.commit().await?;
        return Ok(BatchClaimOutcome::UnknownSlot);
    };
    let (Some(reservation_id), Some(bound_usd)) = (reservation_id, bound_usd) else {
        tx.commit().await?;
        return Ok(BatchClaimOutcome::NotEligible {
            reason: "free slot is never reserved".to_string(),
        });
    };
    if let Some(claimed_session_id) = claimed_session_id {
        tx.commit().await?;
        return Ok(BatchClaimOutcome::AlreadyClaimed {
            reservation_id,
            claimed_session_id,
        });
    }
    let descends: bool = sqlx::query_scalar(
        "WITH RECURSIVE lineage(id, parent_id) AS (
             SELECT id, parent_session_id FROM sessions WHERE id = ?
             UNION ALL
             SELECT s.id, s.parent_session_id
             FROM sessions s JOIN lineage l ON s.id = l.parent_id
         )
         SELECT EXISTS(SELECT 1 FROM lineage WHERE id = ?)",
    )
    .bind(child_session_id)
    .bind(&owner)
    .fetch_one(&mut *tx)
    .await?;
    if !descends {
        tx.commit().await?;
        return Ok(BatchClaimOutcome::NotEligible {
            reason: "child session does not descend from the batch owner".to_string(),
        });
    }
    let child_extension: String =
        sqlx::query_scalar("SELECT extension_data FROM sessions WHERE id = ?")
            .bind(child_session_id)
            .fetch_one(&mut *tx)
            .await?;
    let child_task = serde_json::from_str::<ExtensionData>(&child_extension)
        .ok()
        .and_then(|data| budget_task_id(&data));
    if child_task.as_deref() != Some(task_id.as_str()) {
        tx.commit().await?;
        return Ok(BatchClaimOutcome::NotEligible {
            reason: "child session carries a different budget task".to_string(),
        });
    }
    // Settlement requires the reservation's session to equal the ledger
    // row's, so attribution moves to the child that will make the call.
    let moved = sqlx::query(
        "UPDATE cost_reservations SET session_id = ?, updated_at = ?
         WHERE reservation_id = ? AND task_id = ? AND state = 'pending'",
    )
    .bind(child_session_id)
    .bind(&now_s)
    .bind(&reservation_id)
    .bind(&task_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if moved != 1 {
        tx.commit().await?;
        return Ok(BatchClaimOutcome::NotEligible {
            reason: "slot reservation is no longer pending".to_string(),
        });
    }
    sqlx::query(
        "UPDATE cost_reservation_batch_members SET claimed_session_id = ?, claimed_at = ?
         WHERE batch_id = ? AND slot_id = ? AND claimed_session_id IS NULL",
    )
    .bind(child_session_id)
    .bind(&now_s)
    .bind(batch_id)
    .bind(slot_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(BatchClaimOutcome::Claimed {
        reservation_id,
        bound_usd,
    })
}

async fn finish_batch(
    pool: &Pool<Sqlite>,
    batch_id: &str,
    target_status: &str,
) -> Result<BatchCloseSummary, BatchAdmissionError> {
    let now_s = Utc::now().to_rfc3339();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let Some(summary) = release_batch_in_tx(&mut tx, batch_id, target_status, &now_s).await? else {
        return Err(BatchAdmissionError::UnknownBatch(batch_id.to_string()));
    };
    tx.commit().await?;
    Ok(summary)
}

/// Idempotent close: releases only never-claimed slots, retains claimed ones.
pub(crate) async fn close_reservation_batch(
    pool: &Pool<Sqlite>,
    batch_id: &str,
) -> Result<BatchCloseSummary, BatchAdmissionError> {
    finish_batch(pool, batch_id, "closed").await
}

/// Stops new claims and releases unclaimed slots; never refunds a claimed one.
pub(crate) async fn expire_reservation_batch(
    pool: &Pool<Sqlite>,
    batch_id: &str,
) -> Result<BatchCloseSummary, BatchAdmissionError> {
    finish_batch(pool, batch_id, "expired").await
}

/// Expire every lapsed open batch. Runs on startup and inside admission.
pub(crate) async fn reconcile_expired_batches(
    pool: &Pool<Sqlite>,
) -> Result<BatchExpirySweep, BatchAdmissionError> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let sweep = reconcile_expired_batches_in_tx(&mut tx, Utc::now()).await?;
    tx.commit().await?;
    Ok(sweep)
}

#[cfg(test)]
mod tests;
