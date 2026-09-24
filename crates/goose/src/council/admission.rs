//! Council's batch seam. Production budget arithmetic belongs to K1b.
use crate::cost_router::budget::BudgetScope;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct ConvenePlannedSlot {
    pub slot_id: String,
    pub invocation_id: String,
    pub provider: String,
    pub model: Option<String>,
    /// None is a free slot: recorded in the plan, never reserved.
    pub bound_usd: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ConveneGateApproval {
    pub decision_id: String,
    pub plan_hash: String,
}

#[derive(Debug, Clone)]
pub struct ConveneBatchRequest {
    pub request_id: String,
    pub plan_hash: String,
    pub owner_session_id: String,
    pub slots: Vec<ConvenePlannedSlot>,
    pub lease_until: String,
    pub gate_approval: Option<ConveneGateApproval>,
}

#[derive(Debug, Clone)]
pub enum ConveneAdmissionOutcome {
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

#[derive(Debug, Clone)]
pub enum ConveneClaimOutcome {
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

#[derive(Debug, Clone, Default)]
pub struct ConveneCloseSummary {
    pub released_slots: usize,
    pub released_usd: f64,
    pub retained_slots: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ConveneAdmissionError(pub String);

#[async_trait]
pub trait ConveneAdmission: Send + Sync {
    async fn admit(
        &self,
        req: ConveneBatchRequest,
    ) -> Result<ConveneAdmissionOutcome, ConveneAdmissionError>;
    async fn claim(
        &self,
        batch_id: &str,
        slot_id: &str,
        child_session_id: &str,
    ) -> Result<ConveneClaimOutcome, ConveneAdmissionError>;
    async fn close(&self, batch_id: &str) -> Result<ConveneCloseSummary, ConveneAdmissionError>;
    async fn expire(&self, batch_id: &str) -> Result<ConveneCloseSummary, ConveneAdmissionError>;
}

/// Transitional behavior only: each LiveCaller invocation still reserves itself.
/// This is deliberately NOT an aggregate authorization or a budget precheck.
pub struct LegacyAdmission;

#[async_trait]
impl ConveneAdmission for LegacyAdmission {
    async fn admit(
        &self,
        req: ConveneBatchRequest,
    ) -> Result<ConveneAdmissionOutcome, ConveneAdmissionError> {
        let bound: f64 = req.slots.iter().filter_map(|s| s.bound_usd).sum();
        tracing::info!(council_admission = "legacy_no_envelope", request_id = %req.request_id,
            plan_hash = %req.plan_hash, B = bound, "Council admission has no batch envelope");
        Ok(ConveneAdmissionOutcome::Admitted {
            batch_id: req.request_id,
            reserved_usd: 0.0,
            paid_slots: 0,
        })
    }

    async fn claim(
        &self,
        _batch_id: &str,
        _slot_id: &str,
        _child_session_id: &str,
    ) -> Result<ConveneClaimOutcome, ConveneAdmissionError> {
        Ok(ConveneClaimOutcome::NotEligible {
            reason: "legacy_no_envelope".into(),
        })
    }
    async fn close(&self, _batch_id: &str) -> Result<ConveneCloseSummary, ConveneAdmissionError> {
        Ok(ConveneCloseSummary::default())
    }
    async fn expire(&self, batch_id: &str) -> Result<ConveneCloseSummary, ConveneAdmissionError> {
        self.close(batch_id).await
    }
}

/// A deterministic contract double, not a second production budget engine.
#[cfg(test)]
pub mod scripted {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    pub struct ScriptedAdmission {
        pub soft_ceiling: f64,
        pub hard_ceiling: f64,
        state: Mutex<State>,
    }
    #[derive(Default)]
    struct State {
        batches: HashMap<String, Batch>,
        consumed: HashSet<String>,
    }
    struct Batch {
        request: ConveneBatchRequest,
        claims: HashMap<String, String>,
        closed: bool,
        expired: bool,
    }
    impl ScriptedAdmission {
        pub fn new(soft_ceiling: f64, hard_ceiling: f64) -> Self {
            Self {
                soft_ceiling,
                hard_ceiling,
                state: Mutex::new(State::default()),
            }
        }
        fn release(&self, batch_id: &str, expired: bool) -> ConveneCloseSummary {
            let mut state = self.state.lock().unwrap();
            let Some(batch) = state.batches.get_mut(batch_id) else {
                return ConveneCloseSummary::default();
            };
            let mut summary = ConveneCloseSummary::default();
            for slot in &batch.request.slots {
                if let Some(bound) = slot.bound_usd {
                    if batch.claims.contains_key(&slot.slot_id) {
                        summary.retained_slots += 1;
                    } else if !batch.closed && !batch.expired {
                        summary.released_slots += 1;
                        summary.released_usd += bound;
                    }
                }
            }
            batch.closed |= !expired;
            batch.expired |= expired;
            summary
        }
    }
    #[async_trait]
    impl ConveneAdmission for ScriptedAdmission {
        async fn admit(
            &self,
            req: ConveneBatchRequest,
        ) -> Result<ConveneAdmissionOutcome, ConveneAdmissionError> {
            let mut state = self.state.lock().unwrap();
            if let Some(batch) = state.batches.get(&req.request_id) {
                return Ok(if batch.request.plan_hash == req.plan_hash {
                    ConveneAdmissionOutcome::AlreadyAdmitted {
                        batch_id: req.request_id,
                    }
                } else {
                    ConveneAdmissionOutcome::PlanChanged {
                        batch_id: req.request_id,
                        admitted_plan_hash: batch.request.plan_hash.clone(),
                    }
                });
            }
            let amount: f64 = req.slots.iter().filter_map(|s| s.bound_usd).sum();
            if req
                .slots
                .iter()
                .filter_map(|s| s.bound_usd)
                .any(|b| !b.is_finite() || b <= 0.0)
                || !amount.is_finite()
            {
                return Ok(ConveneAdmissionOutcome::Unknown {
                    reason: "invalid scripted bound".into(),
                });
            }
            if amount > self.hard_ceiling {
                return Ok(ConveneAdmissionOutcome::Refused {
                    scope: BudgetScope::Task,
                    spent_usd: 0.0,
                    held_usd: 0.0,
                    requested_usd: amount,
                    ceiling_usd: self.hard_ceiling,
                });
            }
            let approved = req.gate_approval.as_ref().is_some_and(|a| {
                a.plan_hash == req.plan_hash && !state.consumed.contains(&a.decision_id)
            });
            if amount > self.soft_ceiling && !approved {
                return Ok(ConveneAdmissionOutcome::NeedsGate {
                    scope: BudgetScope::Task,
                    spent_usd: 0.0,
                    held_usd: 0.0,
                    requested_usd: amount,
                    ceiling_usd: self.soft_ceiling,
                });
            }
            if approved {
                state
                    .consumed
                    .insert(req.gate_approval.as_ref().unwrap().decision_id.clone());
            }
            let paid_slots = req.slots.iter().filter(|s| s.bound_usd.is_some()).count();
            let batch_id = req.request_id.clone();
            state.batches.insert(
                batch_id.clone(),
                Batch {
                    request: req,
                    claims: HashMap::new(),
                    closed: false,
                    expired: false,
                },
            );
            Ok(ConveneAdmissionOutcome::Admitted {
                batch_id,
                reserved_usd: amount,
                paid_slots,
            })
        }
        async fn claim(
            &self,
            batch_id: &str,
            slot_id: &str,
            child_session_id: &str,
        ) -> Result<ConveneClaimOutcome, ConveneAdmissionError> {
            let mut state = self.state.lock().unwrap();
            let Some(batch) = state.batches.get_mut(batch_id) else {
                return Ok(ConveneClaimOutcome::UnknownSlot);
            };
            if batch.expired || batch.request.lease_until <= chrono::Utc::now().to_rfc3339() {
                return Ok(ConveneClaimOutcome::BatchExpired);
            }
            if batch.closed {
                return Ok(ConveneClaimOutcome::BatchClosed);
            }
            let Some(slot) = batch.request.slots.iter().find(|s| s.slot_id == slot_id) else {
                return Ok(ConveneClaimOutcome::UnknownSlot);
            };
            let Some(bound_usd) = slot.bound_usd else {
                return Ok(ConveneClaimOutcome::NotEligible {
                    reason: "free slot is never reserved".into(),
                });
            };
            let reservation_id = slot.invocation_id.clone();
            if let Some(child) = batch.claims.get(slot_id) {
                return Ok(ConveneClaimOutcome::AlreadyClaimed {
                    reservation_id,
                    claimed_session_id: child.clone(),
                });
            }
            batch.claims.insert(slot_id.into(), child_session_id.into());
            Ok(ConveneClaimOutcome::Claimed {
                reservation_id,
                bound_usd,
            })
        }
        async fn close(
            &self,
            batch_id: &str,
        ) -> Result<ConveneCloseSummary, ConveneAdmissionError> {
            Ok(self.release(batch_id, false))
        }
        async fn expire(
            &self,
            batch_id: &str,
        ) -> Result<ConveneCloseSummary, ConveneAdmissionError> {
            Ok(self.release(batch_id, true))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::scripted::ScriptedAdmission;
    use super::*;

    fn request(id: &str, hash: &str) -> ConveneBatchRequest {
        ConveneBatchRequest {
            request_id: id.into(),
            plan_hash: hash.into(),
            owner_session_id: "owner".into(),
            slots: (0..24)
                .map(|i| ConvenePlannedSlot {
                    slot_id: i.to_string(),
                    invocation_id: format!("invocation-{i}"),
                    provider: "openai".into(),
                    model: Some("gpt-4o".into()),
                    bound_usd: Some(0.25),
                })
                .collect(),
            lease_until: (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339(),
            gate_approval: None,
        }
    }
    #[tokio::test]
    async fn aggregate_gate_is_atomic_and_approval_is_consumed_once() {
        let admission = ScriptedAdmission::new(1.0, 10.0);
        let mut req = request("one", "hash");
        assert!(matches!(
            admission.admit(req.clone()).await.unwrap(),
            ConveneAdmissionOutcome::NeedsGate {
                requested_usd: 6.0,
                ..
            }
        ));
        assert!(matches!(
            admission.claim("one", "0", "child").await.unwrap(),
            ConveneClaimOutcome::UnknownSlot
        ));
        req.gate_approval = Some(ConveneGateApproval {
            decision_id: "approval".into(),
            plan_hash: "hash".into(),
        });
        assert!(matches!(
            admission.admit(req.clone()).await.unwrap(),
            ConveneAdmissionOutcome::Admitted { paid_slots: 24, .. }
        ));
        assert!(matches!(
            admission.admit(req.clone()).await.unwrap(),
            ConveneAdmissionOutcome::AlreadyAdmitted { .. }
        ));
        req.plan_hash = "changed".into();
        assert!(matches!(
            admission.admit(req.clone()).await.unwrap(),
            ConveneAdmissionOutcome::PlanChanged { .. }
        ));
        req.request_id = "two".into();
        req.plan_hash = "hash".into();
        assert!(matches!(
            admission.admit(req).await.unwrap(),
            ConveneAdmissionOutcome::NeedsGate { .. }
        ));
    }
    #[tokio::test]
    async fn hard_limit_cannot_be_approved_and_close_retains_claims() {
        let admission = ScriptedAdmission::new(1.0, 5.0);
        let mut req = request("one", "hash");
        req.gate_approval = Some(ConveneGateApproval {
            decision_id: "approval".into(),
            plan_hash: "hash".into(),
        });
        assert!(matches!(
            admission.admit(req).await.unwrap(),
            ConveneAdmissionOutcome::Refused { .. }
        ));
        let admission = ScriptedAdmission::new(10.0, 20.0);
        let mut req = request("one", "hash");
        req.slots.last_mut().unwrap().bound_usd = None;
        admission.admit(req).await.unwrap();
        assert!(
            matches!(admission.claim("one", "0", "child").await.unwrap(), ConveneClaimOutcome::Claimed { reservation_id, .. } if reservation_id == "invocation-0")
        );
        assert!(
            matches!(admission.claim("one", "0", "other").await.unwrap(), ConveneClaimOutcome::AlreadyClaimed { claimed_session_id, .. } if claimed_session_id == "child")
        );
        assert!(matches!(
            admission.claim("one", "23", "child").await.unwrap(),
            ConveneClaimOutcome::NotEligible { .. }
        ));
        let closed = admission.close("one").await.unwrap();
        assert_eq!(
            (
                closed.released_slots,
                closed.retained_slots,
                closed.released_usd
            ),
            (22, 1, 5.5)
        );
        assert_eq!(admission.close("one").await.unwrap().released_slots, 0);
        assert!(matches!(
            admission.claim("one", "1", "child").await.unwrap(),
            ConveneClaimOutcome::BatchClosed
        ));
    }
}
