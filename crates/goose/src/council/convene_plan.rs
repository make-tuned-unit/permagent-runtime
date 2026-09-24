//! Freeze all logical calls before admission. This module only sums shared bounds.
use super::{admission::ConvenePlannedSlot, debate::MemberCaller, membership::Member};
use crate::{model::ModelConfig, session::CostTier};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    Round1,
    Round2,
    Chair,
    Verdict,
}

#[derive(Debug, Clone)]
pub struct ConveneSlot {
    pub(crate) kind: CallKind,
    pub(crate) planned: ConvenePlannedSlot,
    pub(crate) config: ModelConfig,
    pub(crate) tier: CostTier,
    pub(crate) max_physical_attempts: u32,
}

#[derive(Debug, Clone)]
pub struct ConvenePlan {
    pub(crate) request_id: String,
    pub(crate) plan_fingerprint: String,
    pub(crate) members: Vec<Member>,
    pub(crate) chair: Member,
    pub(crate) slots: Vec<ConveneSlot>,
    pub(crate) bound_usd: f64,
}

/// Canonical JSON also orders request_params maps; HashMap iteration must not
/// invalidate yesterday's approval after a restart.
pub fn fingerprint(value: &serde_json::Value) -> String {
    fn feed(hash: &mut Sha256, value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                hash.update(b"{");
                let mut keys: Vec<_> = object.keys().collect();
                keys.sort();
                for key in keys {
                    hash.update(serde_json::to_string(key).expect("JSON key"));
                    hash.update(b":");
                    feed(hash, &object[key]);
                    hash.update(b",");
                }
                hash.update(b"}");
            }
            serde_json::Value::Array(array) => {
                hash.update(b"[");
                for value in array {
                    feed(hash, value);
                    hash.update(b",");
                }
                hash.update(b"]");
            }
            _ => hash.update(value.to_string()),
        }
    }
    let mut hash = Sha256::new();
    feed(&mut hash, value);
    hex::encode(hash.finalize())
}

impl ConvenePlan {
    pub async fn build(
        request_id: String,
        mut members: Vec<Member>,
        chair: Member,
        caller: &dyn MemberCaller,
    ) -> Result<Self, String> {
        members.sort_by(|a, b| (&a.provider, &a.model).cmp(&(&b.provider, &b.model)));
        if members.is_empty() {
            return Err("no connected chat providers sit on the Council (connect a key, or un-exclude a seat)".into());
        }
        if members
            .iter()
            .chain(std::iter::once(&chair))
            .any(|m| !super::membership::is_council_eligible_provider(&m.provider))
        {
            return Err("reserved test providers cannot sit on or chair the Council".into());
        }
        let mut slots = Vec::new();
        for (index, member) in members.iter().chain(std::iter::once(&chair)).enumerate() {
            let (mut config, declared_tier, attempts) = caller.plan_model(member).await?;
            let tier = crate::cost_router::reservation::effective_cost_tier(
                &member.provider,
                &member.model,
                declared_tier,
            );
            let bound = crate::cost_router::plan_reservation_bound(
                &member.provider,
                &member.model,
                tier,
                &config,
                attempts,
            )
            .map_err(|e| format!("council provider call is not priced safely: {e}"))?;
            if let Some(bound) = bound {
                config.max_tokens =
                    Some(i32::try_from(bound.output_tokens).map_err(|e| e.to_string())?);
            }
            let kinds = if index == members.len() {
                [CallKind::Chair, CallKind::Verdict]
            } else {
                [CallKind::Round1, CallKind::Round2]
            };
            for kind in kinds {
                let slot_id = format!("{kind:?}:{index}");
                slots.push(ConveneSlot {
                    kind,
                    planned: ConvenePlannedSlot {
                        invocation_id: fingerprint(&serde_json::json!([request_id, slot_id])),
                        slot_id,
                        provider: member.provider.clone(),
                        model: Some(member.model.clone()),
                        bound_usd: bound.map(|b| b.amount_usd),
                    },
                    config: config.clone(),
                    tier,
                    max_physical_attempts: bound
                        .map(|b| b.max_physical_attempts)
                        .unwrap_or(attempts),
                });
            }
        }
        let bound_usd = slots
            .iter()
            .filter_map(|s| s.planned.bound_usd)
            .sum::<f64>();
        if !bound_usd.is_finite() {
            return Err("Council plan bound is not finite".into());
        }
        let fields: Vec<_> = slots
            .iter()
            .map(|s| {
                serde_json::json!({
                    "kind": s.kind, "slot_id": s.planned.slot_id, "provider": s.planned.provider,
                    "model": s.planned.model, "config": s.config, "tier": s.tier.as_str(),
                    "attempts": s.max_physical_attempts, "bound": s.planned.bound_usd,
                })
            })
            .collect();
        let plan_fingerprint =
            fingerprint(&serde_json::json!({"members": members, "chair": chair, "slots": fields}));
        Ok(Self {
            request_id,
            plan_fingerprint,
            members,
            chair,
            slots,
            bound_usd,
        })
    }
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
    pub fn plan_fingerprint(&self) -> &str {
        &self.plan_fingerprint
    }
    pub fn bound_usd(&self) -> f64 {
        self.bound_usd
    }
}
