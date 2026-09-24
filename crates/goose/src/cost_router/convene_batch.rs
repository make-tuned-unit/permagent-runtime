//! K1b: the production Council batch admission.
//!
//! [`DurableBatchAdmission`] implements K1a's `ConveneAdmission` seam by
//! delegating to the SessionManager batch entry points
//! (`session::reservation_batch`). It adds no arithmetic of its own: the
//! frozen plan is admitted as ordinary `pending` reservations with the
//! single-invocation ceiling math applied to the SUM of the paid slots, a
//! claim hands the dispatching child the EXISTING reservation, and close /
//! expiry release only never-claimed slots.
//!
//! Approval failures (`ApprovalConsumed`, `ApprovalInvalid`,
//! `ApprovalUnauthorized`) have no variant in `ConveneAdmissionOutcome`, so
//! they surface as `ConveneAdmissionError` carrying the typed code as the
//! message prefix (`council_approval_consumed: …` etc.). An error admits
//! nothing and never raises a new card, so a replayed human approval can
//! never silently succeed.
use crate::cost_router::budget::BudgetConfig;
use crate::council::admission::{
    ConveneAdmission, ConveneAdmissionError, ConveneAdmissionOutcome, ConveneBatchRequest,
    ConveneClaimOutcome, ConveneCloseSummary,
};
use crate::session::reservation_batch::{
    BatchAdmissionError, BatchAdmissionOutcome, BatchAdmissionRequest, BatchClaimOutcome,
    BatchCloseSummary, BatchGateApproval, BatchSlot,
};
use crate::session::SessionManager;
use async_trait::async_trait;
use std::sync::Arc;

pub struct DurableBatchAdmission {
    manager: Arc<SessionManager>,
    config: BudgetConfig,
}

impl DurableBatchAdmission {
    pub fn new(manager: Arc<SessionManager>, config: BudgetConfig) -> Self {
        Self { manager, config }
    }
}

fn admission_error(error: BatchAdmissionError) -> ConveneAdmissionError {
    ConveneAdmissionError(error.to_string())
}

fn batch_request(req: ConveneBatchRequest) -> BatchAdmissionRequest {
    BatchAdmissionRequest {
        request_id: req.request_id,
        plan_hash: req.plan_hash,
        owner_session_id: req.owner_session_id,
        slots: req
            .slots
            .into_iter()
            .map(|slot| BatchSlot {
                slot_id: slot.slot_id,
                invocation_id: slot.invocation_id,
                provider: slot.provider,
                model: slot.model,
                bound_usd: slot.bound_usd,
            })
            .collect(),
        lease_until: req.lease_until,
        gate_approval: req.gate_approval.map(|approval| BatchGateApproval {
            decision_id: approval.decision_id,
            plan_hash: approval.plan_hash,
        }),
    }
}

fn close_summary(summary: BatchCloseSummary) -> ConveneCloseSummary {
    ConveneCloseSummary {
        released_slots: summary.released_slots,
        released_usd: summary.released_usd,
        retained_slots: summary.retained_slots,
    }
}

#[async_trait]
impl ConveneAdmission for DurableBatchAdmission {
    async fn admit(
        &self,
        req: ConveneBatchRequest,
    ) -> Result<ConveneAdmissionOutcome, ConveneAdmissionError> {
        let outcome = self
            .manager
            .reserve_invocation_batch(&batch_request(req), &self.config)
            .await
            .map_err(admission_error)?;
        Ok(match outcome {
            BatchAdmissionOutcome::Admitted {
                batch_id,
                reserved_usd,
                paid_slots,
            } => ConveneAdmissionOutcome::Admitted {
                batch_id,
                reserved_usd,
                paid_slots,
            },
            BatchAdmissionOutcome::AlreadyAdmitted { batch_id } => {
                ConveneAdmissionOutcome::AlreadyAdmitted { batch_id }
            }
            BatchAdmissionOutcome::PlanChanged {
                batch_id,
                admitted_plan_hash,
            } => ConveneAdmissionOutcome::PlanChanged {
                batch_id,
                admitted_plan_hash,
            },
            BatchAdmissionOutcome::NeedsGate {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
            } => ConveneAdmissionOutcome::NeedsGate {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
            },
            BatchAdmissionOutcome::Refused {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
            } => ConveneAdmissionOutcome::Refused {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
            },
            BatchAdmissionOutcome::Unknown { reason } => {
                ConveneAdmissionOutcome::Unknown { reason }
            }
        })
    }

    async fn claim(
        &self,
        batch_id: &str,
        slot_id: &str,
        child_session_id: &str,
    ) -> Result<ConveneClaimOutcome, ConveneAdmissionError> {
        let outcome = self
            .manager
            .claim_batch_invocation(batch_id, slot_id, child_session_id)
            .await
            .map_err(admission_error)?;
        Ok(match outcome {
            BatchClaimOutcome::Claimed {
                reservation_id,
                bound_usd,
            } => ConveneClaimOutcome::Claimed {
                reservation_id,
                bound_usd,
            },
            BatchClaimOutcome::AlreadyClaimed {
                reservation_id,
                claimed_session_id,
            } => ConveneClaimOutcome::AlreadyClaimed {
                reservation_id,
                claimed_session_id,
            },
            BatchClaimOutcome::BatchClosed => ConveneClaimOutcome::BatchClosed,
            BatchClaimOutcome::BatchExpired => ConveneClaimOutcome::BatchExpired,
            BatchClaimOutcome::NotEligible { reason } => {
                ConveneClaimOutcome::NotEligible { reason }
            }
            BatchClaimOutcome::UnknownSlot => ConveneClaimOutcome::UnknownSlot,
        })
    }

    async fn close(&self, batch_id: &str) -> Result<ConveneCloseSummary, ConveneAdmissionError> {
        self.manager
            .close_reservation_batch(batch_id)
            .await
            .map(close_summary)
            .map_err(admission_error)
    }

    async fn expire(&self, batch_id: &str) -> Result<ConveneCloseSummary, ConveneAdmissionError> {
        self.manager
            .expire_reservation_batch(batch_id)
            .await
            .map(close_summary)
            .map_err(admission_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost_router::budget::BudgetCeilings;
    use crate::council::admission::{ConveneGateApproval, ConvenePlannedSlot};

    /// The adapter maps the trait contract onto the durable store faithfully:
    /// admission writes ordinary reservations, a claim returns an existing id,
    /// and an approval error is a typed error, never an admission.
    #[tokio::test]
    async fn durable_admission_implements_the_convene_contract() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let owner = manager
            .create_session(
                dir.path().to_path_buf(),
                "Council convene".into(),
                crate::session::SessionType::SubAgent,
                crate::config::GooseMode::Auto,
            )
            .await
            .unwrap();
        manager.begin_budget_task(&owner.id).await.unwrap();
        let ceilings = BudgetCeilings {
            soft: 10.0,
            gate: 10.0,
            hard: 20.0,
        };
        let admission = DurableBatchAdmission::new(
            manager.clone(),
            BudgetConfig {
                task: ceilings,
                session: ceilings,
            },
        );
        let request = ConveneBatchRequest {
            request_id: "adapter-req".into(),
            plan_hash: "hash".into(),
            owner_session_id: owner.id.clone(),
            slots: (0..3)
                .map(|i| ConvenePlannedSlot {
                    slot_id: format!("Round1:{i}"),
                    invocation_id: format!("adapter-invocation-{i}"),
                    provider: "openai".into(),
                    model: Some("gpt-4o".into()),
                    bound_usd: Some(0.5),
                })
                .collect(),
            lease_until: (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339(),
            gate_approval: None,
        };
        let ConveneAdmissionOutcome::Admitted {
            batch_id,
            paid_slots,
            ..
        } = admission.admit(request.clone()).await.unwrap()
        else {
            panic!("expected admission");
        };
        assert_eq!(paid_slots, 3);
        let pool = manager.pool_clone().await.unwrap();
        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(before, 3);
        let child = manager
            .create_session_with_parent(
                Some(&owner.id),
                dir.path().to_path_buf(),
                "Council Round1:0".into(),
                crate::session::SessionType::SubAgent,
                crate::config::GooseMode::Auto,
            )
            .await
            .unwrap();
        let ConveneClaimOutcome::Claimed { reservation_id, .. } = admission
            .claim(&batch_id, "Round1:0", &child.id)
            .await
            .unwrap()
        else {
            panic!("expected claim");
        };
        assert_eq!(reservation_id, "invocation:adapter-invocation-0");
        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(after, before);

        let summary = admission.close(&batch_id).await.unwrap();
        assert_eq!((summary.released_slots, summary.retained_slots), (2, 1));

        // A reference to a decision that does not exist is a typed error.
        let mut forged = request;
        forged.request_id = "adapter-forged".into();
        forged.slots.iter_mut().enumerate().for_each(|(i, s)| {
            s.invocation_id = format!("adapter-forged-{i}");
        });
        forged.gate_approval = Some(ConveneGateApproval {
            decision_id: "no-such-decision".into(),
            plan_hash: "hash".into(),
        });
        let error = admission.admit(forged).await.unwrap_err();
        assert!(
            error.0.starts_with("council_approval_invalid:"),
            "{}",
            error.0
        );
    }
}
