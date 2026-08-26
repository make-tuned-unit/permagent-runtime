//! Delegate routing — the model a delegated subagent actually runs on.
//!
//! ## Why this module exists (measured, 2026-08-25)
//!
//! The model-defaults bench ran the coding harness (`permagent run --recipe
//! permagent-coding`) with every `PERMAGENT_PACK_{EDIT,HARD,MECHANICAL,LOCAL}_
//! {PROVIDER,MODEL}` key pinned to the candidate model, and measured the
//! subagents that `delegate`/`summon` spawned running on `anthropic/claude-fable-5`
//! anyway: a `gpt-5.4-mini` run billed three Fable calls at $1.51 against $0.31 of
//! its own spend, and a `glm-5.3` run billed six at $2.20 — 82% of that task's
//! total. Nobody asked for those calls.
//!
//! Two independent defects produced that:
//!
//! 1. **The pins were never read.** `summon::resolve_provider` consulted
//!    `PERMAGENT_ROLE_*` ([`super::role_map`]) and nothing else in that family.
//!    The `PERMAGENT_PACK_*` keys ([`super::packs`]) fed only the internal tier
//!    ladder and `permagent-eval`, so eight deliberately-set operator keys had no
//!    effect at all on where a delegate ran.
//! 2. **The derived map escalated silently, across providers.** With no
//!    `PERMAGENT_ROLE_*` pin, dispatch fell to the recommender-DERIVED best-fit map
//!    ([`super::derived`]). That map is built from every model of every KEYED
//!    provider, so merely holding an `ANTHROPIC_API_KEY` puts the whole Anthropic
//!    line into the pool; `recommend::edit_rank` then ranks by
//!    `edit_format_reliability` FIRST with cost only as a tiebreak, and
//!    `claude-fable-5` holds the knowledge base's maximum (0.985) — as it does for
//!    orchestration (0.95). So EDIT, ORCHESTRATE and REVIEW delegates all landed on
//!    the single most expensive Anthropic row, in a session the operator had
//!    pointed at a different provider entirely.
//!
//! ## The rule
//!
//! **A subagent never silently runs on a model family or provider the operator did
//! not configure for this session.** Cost-router escalation is opt-in
//! ([`KEY_ALLOW_ESCALATION`], default off), and even when it is on it must stay
//! inside the operator's configured providers, fail closed against the spend caps,
//! and leave a receipt naming the model and why.
//!
//! ## Precedence (as shipped)
//!
//! | # | Source | Where it comes from |
//! |---|--------|---------------------|
//! | 1 | explicit call param | `delegate`'s own `provider`/`model` arguments |
//! | 2 | recipe setting | the delegate recipe's `settings.goose_provider/goose_model` |
//! | 3 | role pin | `PERMAGENT_ROLE_{ROLE}_{PROVIDER,MODEL}` |
//! | 4 | pack pin | `PERMAGENT_PACK_{EDIT,HARD,MECHANICAL,LOCAL}_{PROVIDER,MODEL}` |
//! | 5 | escalation | the derived best-fit map — ONLY with [`KEY_ALLOW_ESCALATION`] on, the target provider configured, and the spend band `Ok` |
//! | 6 | session | the session's own provider+model (`GOOSE_PROVIDER`/`GOOSE_MODEL` when the session carries none) |
//!
//! Levels 1–2 are applied by `summon` itself (they are per-call, not per-role);
//! this module decides levels 3–6 and hands the result to `summon`'s existing
//! consistency reconciliation. Level 6 is the FALLBACK, not a floor: it is what a
//! refused escalation lands on, which is the whole point — the operator's own model
//! is always a safe answer, and Anthropic's most expensive tier is not.
//!
//! Pure core ([`decide_delegate_model`]) plus a thin config wrapper
//! ([`delegate_routing`]), mirroring [`super::budget`] and [`super::role_map`].

use serde::Serialize;

use super::budget::BudgetBand;
use super::derived::{DerivedRoleMap, Provenance};
use super::packs::{pack_role_for_workflow_role, ModelPack};
use super::recommend::WorkflowRole;
use super::role_map::RoleModel;

/// Config/env key for the escalation knob. `false` (the default) means a
/// delegate never leaves the operator's pins and session model on the cost
/// router's own judgment.
pub const KEY_ALLOW_ESCALATION: &str = "PERMAGENT_DELEGATE_ALLOW_ESCALATION";

/// Which precedence level chose the delegate's model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegateSource {
    /// A hand-configured `PERMAGENT_ROLE_*` mapping.
    RolePin,
    /// An explicitly-set `PERMAGENT_PACK_*` pin.
    PackPin,
    /// The cost router's derived best-fit pick, with escalation allowed and every
    /// gate passed.
    Escalated,
    /// The session's own provider+model — the fallback, and what a refused
    /// escalation lands on.
    Session,
}

impl DelegateSource {
    pub fn as_str(self) -> &'static str {
        match self {
            DelegateSource::RolePin => "role_pin",
            DelegateSource::PackPin => "pack_pin",
            DelegateSource::Escalated => "escalated",
            DelegateSource::Session => "session",
        }
    }
}

/// Why an offered escalation was not taken. Present only when the derived map
/// actually offered something — an empty map is an absence, not a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationRefusal {
    /// [`KEY_ALLOW_ESCALATION`] is off. The default, and the one that stops the
    /// measured leak.
    Disabled,
    /// The escalation target's provider is not configured for this operator.
    UnconfiguredProvider,
    /// Spend is at or past a ceiling band. Fail closed: an unknown band refuses
    /// too, because unmeasurable spend must make the cap fire early, never late
    /// (the same discipline as [`super::budget::budget_verdict_with_unpriced`]).
    SpendCapped,
}

impl EscalationRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            EscalationRefusal::Disabled => "escalation_disabled",
            EscalationRefusal::UnconfiguredProvider => "provider_not_configured",
            EscalationRefusal::SpendCapped => "spend_capped",
        }
    }

    /// The clause a receipt uses to say why.
    fn because(self) -> String {
        match self {
            EscalationRefusal::Disabled => {
                format!("{KEY_ALLOW_ESCALATION} is off")
            }
            EscalationRefusal::UnconfiguredProvider => {
                "its provider is not configured for this operator".to_string()
            }
            EscalationRefusal::SpendCapped => "spend is at a ceiling".to_string(),
        }
    }
}

/// The routing decision for one delegate dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct DelegateRouting {
    /// The provider+model to route to, or `None` to inherit the session pair.
    /// `None` is ALWAYS the session, never a vendor default.
    pub role_model: Option<RoleModel>,
    pub source: DelegateSource,
    /// The escalation the router wanted and did not get, if any.
    pub refused: Option<(RoleModel, EscalationRefusal)>,
    /// One line naming the model and why — for the status row, the routing
    /// snapshot and the log.
    pub receipt: String,
    /// The workflow role this decision was made for, carried for the receipt.
    pub role: Option<WorkflowRole>,
}

impl DelegateRouting {
    /// The routing snapshot payload (`model_routing`), the same receipt shape the
    /// goal engine writes onto a card. Mirrors
    /// [`super::derived::model_routing_receipt`], plus the escalation verdict.
    pub fn receipt_json(&self) -> serde_json::Value {
        let mut v = serde_json::json!({
            "role": self.role.map(|r| r.as_str()),
            "source": self.source.as_str(),
            "summary": self.receipt,
        });
        if let Some(obj) = v.as_object_mut() {
            if let Some(rm) = &self.role_model {
                obj.insert("provider".into(), rm.provider.clone().into());
                obj.insert("model".into(), rm.model.clone().into());
            }
            if let Some((rm, why)) = &self.refused {
                obj.insert(
                    "escalation_refused".into(),
                    serde_json::json!({
                        "provider": rm.provider,
                        "model": rm.model,
                        "reason": why.as_str(),
                    }),
                );
            }
        }
        v
    }
}

/// A `(provider, model)` label for the receipt.
fn label(provider: &str, model: &str) -> String {
    format!("{provider}/{model}")
}

fn role_label(role: Option<WorkflowRole>) -> String {
    match role {
        Some(r) => format!("delegate[{}]", r.as_str()),
        None => "delegate".to_string(),
    }
}

/// Pure: decide which model a delegated subagent runs on, at precedence levels
/// 3–6 (see the module table). Levels 1–2 — the explicit call param and the
/// recipe setting — are applied by the caller and are not visible here, because
/// they are per-call operator intent that outranks every role-shaped rule.
///
/// `session` is the session's own `(provider, model)` for the receipt only; the
/// decision never rewrites it, it just declines to leave it.
///
/// Fail-closed: `spend` of `None` (an unknown band) refuses escalation.
#[allow(clippy::too_many_arguments)]
pub fn decide_delegate_model(
    role: Option<WorkflowRole>,
    role_pin: Option<RoleModel>,
    pack_pin: Option<RoleModel>,
    derived: Option<RoleModel>,
    allow_escalation: bool,
    spend: Option<BudgetBand>,
    is_provider_configured: &impl Fn(&str) -> bool,
    session: Option<(&str, &str)>,
) -> DelegateRouting {
    let who = role_label(role);
    let session_label = session
        .map(|(p, m)| label(p, m))
        .unwrap_or_else(|| "the session model".to_string());

    // 3. The hand-configured role pin.
    if let Some(rm) = role_pin {
        let receipt = format!(
            "{who} → {} · role pin ({})",
            label(&rm.provider, &rm.model),
            role.map(|r| format!("PERMAGENT_ROLE_{}_*", r.as_str().to_uppercase()))
                .unwrap_or_else(|| "PERMAGENT_ROLE_*".to_string()),
        );
        return DelegateRouting {
            role_model: Some(rm),
            source: DelegateSource::RolePin,
            refused: None,
            receipt,
            role,
        };
    }

    // 4. The explicitly-set pack pin.
    if let Some(rm) = pack_pin {
        let key = role
            .and_then(pack_role_for_workflow_role)
            .map(|r| format!("PERMAGENT_PACK_{}_*", r.as_str().to_uppercase()))
            .unwrap_or_else(|| "PERMAGENT_PACK_*".to_string());
        let receipt = format!(
            "{who} → {} · pack pin ({key})",
            label(&rm.provider, &rm.model)
        );
        return DelegateRouting {
            role_model: Some(rm),
            source: DelegateSource::PackPin,
            refused: None,
            receipt,
            role,
        };
    }

    // 5. Escalation — opt-in, provider-bounded, spend-gated.
    if let Some(rm) = derived {
        let refusal = if !allow_escalation {
            Some(EscalationRefusal::Disabled)
        } else if !is_provider_configured(&rm.provider) {
            Some(EscalationRefusal::UnconfiguredProvider)
        } else if spend != Some(BudgetBand::Ok) {
            Some(EscalationRefusal::SpendCapped)
        } else {
            None
        };
        return match refusal {
            None => {
                let receipt = format!(
                    "{who} → {} · cost-router escalation (allowed by {KEY_ALLOW_ESCALATION})",
                    label(&rm.provider, &rm.model),
                );
                DelegateRouting {
                    role_model: Some(rm),
                    source: DelegateSource::Escalated,
                    refused: None,
                    receipt,
                    role,
                }
            }
            // 6. Refused ⇒ the session pair, and say what was declined.
            Some(why) => {
                let receipt = format!(
                    "{who} → {session_label} · session model; declined to escalate to {} because {}",
                    label(&rm.provider, &rm.model),
                    why.because(),
                );
                DelegateRouting {
                    role_model: None,
                    source: DelegateSource::Session,
                    refused: Some((rm, why)),
                    receipt,
                    role,
                }
            }
        };
    }

    // 6. Nothing pinned, nothing offered — the session pair.
    let receipt = match role {
        Some(r) => format!(
            "{who} → {session_label} · session model (nothing pinned for the {} role)",
            r.as_str()
        ),
        None => format!("{who} → {session_label} · session model (no workflow role)"),
    };
    DelegateRouting {
        role_model: None,
        source: DelegateSource::Session,
        refused: None,
        receipt,
        role,
    }
}

/// Whether cost-router escalation is allowed for delegates. Default `false`:
/// without an explicit opt-in a delegate never leaves the operator's configured
/// models. Thin config wrapper.
pub fn escalation_allowed() -> bool {
    crate::config::Config::global()
        .get_param::<bool>(KEY_ALLOW_ESCALATION)
        .unwrap_or(false)
}

/// Live [`decide_delegate_model`] against the global config: reads the role pin,
/// the pack pin, the escalation knob, and takes the derived map the caller
/// already fetched. `spend` is the caller's current budget band (`None` when it
/// has not been measured — which refuses escalation).
pub fn delegate_routing(
    role: Option<WorkflowRole>,
    derived: &DerivedRoleMap,
    spend: Option<BudgetBand>,
    session: Option<(&str, &str)>,
) -> DelegateRouting {
    let role_pin = role.and_then(super::role_map::role_model);
    let pack_pin = role
        .and_then(pack_role_for_workflow_role)
        .and_then(super::packs::pack_pin)
        .map(pack_to_role_model);
    let derived_pick = role.and_then(|r| derived.get(r).map(|(rm, _)| rm.clone()));
    decide_delegate_model(
        role,
        role_pin,
        pack_pin,
        derived_pick,
        escalation_allowed(),
        spend,
        &|p| super::recommend::is_provider_configured(p),
        session,
    )
}

/// [`delegate_routing`] over the LIVE derived map, fetched only when a pin has
/// not already decided the answer.
///
/// The derived map probes the local Ollama daemon on a cold cache
/// ([`super::derived::derived_role_map`]); a pinned dispatch must not pay for an
/// answer it will not use, and — more importantly — must not be delayed by a
/// daemon it does not depend on.
pub async fn delegate_routing_live(
    role: Option<WorkflowRole>,
    spend: Option<BudgetBand>,
    session: Option<(&str, &str)>,
) -> DelegateRouting {
    let role_pin = role.and_then(super::role_map::role_model);
    let pack_pin = role
        .and_then(pack_role_for_workflow_role)
        .and_then(super::packs::pack_pin)
        .map(pack_to_role_model);
    let derived_pick = match (role, role_pin.is_none() && pack_pin.is_none()) {
        (Some(r), true) => super::derived::derived_role_map()
            .await
            .get(r)
            .map(|(rm, _)| rm.clone()),
        _ => None,
    };
    decide_delegate_model(
        role,
        role_pin,
        pack_pin,
        derived_pick,
        escalation_allowed(),
        spend,
        &|p| super::recommend::is_provider_configured(p),
        session,
    )
}

/// The derived provenance for a role, for the receipt.
pub fn provenance_for(derived: &DerivedRoleMap, role: Option<WorkflowRole>) -> Option<Provenance> {
    role.and_then(|r| derived.provenance.get(&r).copied())
}

/// A tier pack read as a dispatch target.
pub fn pack_to_role_model(p: ModelPack) -> RoleModel {
    RoleModel {
        provider: p.provider,
        model: p.model,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rm(provider: &str, model: &str) -> RoleModel {
        RoleModel {
            provider: provider.to_string(),
            model: model.to_string(),
        }
    }

    fn all_configured(_: &str) -> bool {
        true
    }

    fn only_openai(p: &str) -> bool {
        p == "openai"
    }

    /// Level 3 beats everything below it.
    #[test]
    fn a_role_pin_wins() {
        let d = decide_delegate_model(
            Some(WorkflowRole::Edit),
            Some(rm("openai", "gpt-5.4-mini")),
            Some(rm("anthropic", "claude-sonnet-5")),
            Some(rm("anthropic", "claude-fable-5")),
            true,
            Some(BudgetBand::Ok),
            &all_configured,
            Some(("openai", "gpt-5.4-mini")),
        );
        assert_eq!(d.source, DelegateSource::RolePin);
        assert_eq!(d.role_model, Some(rm("openai", "gpt-5.4-mini")));
        assert!(d.receipt.contains("PERMAGENT_ROLE_EDIT_*"), "{}", d.receipt);
    }

    /// The measured bug, as a test: the operator pinned every pack to their
    /// candidate model and the delegate ran on `anthropic/claude-fable-5` anyway.
    #[test]
    fn a_pack_pin_is_honoured_over_the_derived_fable_pick() {
        let d = decide_delegate_model(
            Some(WorkflowRole::Edit),
            None,
            Some(rm("openai", "gpt-5.4-mini")),
            Some(rm("anthropic", "claude-fable-5")),
            // Even with escalation ON: a pin is operator intent, not a suggestion.
            true,
            Some(BudgetBand::Ok),
            &all_configured,
            Some(("openai", "gpt-5.4-mini")),
        );
        assert_eq!(d.source, DelegateSource::PackPin);
        assert_eq!(d.role_model, Some(rm("openai", "gpt-5.4-mini")));
        assert!(d.receipt.contains("PERMAGENT_PACK_EDIT_*"), "{}", d.receipt);
        assert!(!d.receipt.contains("fable"), "{}", d.receipt);
    }

    /// With nothing pinned, a delegate stays on the session model — it does not
    /// climb to the strongest thing the operator happens to hold a key for.
    #[test]
    fn no_pins_means_the_session_model_not_an_escalation() {
        let d = decide_delegate_model(
            Some(WorkflowRole::Edit),
            None,
            None,
            Some(rm("anthropic", "claude-fable-5")),
            false,
            Some(BudgetBand::Ok),
            &all_configured,
            Some(("openai", "gpt-5.4-mini")),
        );
        assert_eq!(d.source, DelegateSource::Session);
        assert_eq!(d.role_model, None, "None ⇒ inherit the session pair");
        assert_eq!(
            d.refused,
            Some((
                rm("anthropic", "claude-fable-5"),
                EscalationRefusal::Disabled
            ))
        );
        assert!(d.receipt.contains("declined to escalate"), "{}", d.receipt);
        assert!(
            d.receipt.contains("anthropic/claude-fable-5"),
            "{}",
            d.receipt
        );
    }

    #[test]
    fn no_role_and_no_pins_is_the_session_model() {
        let d = decide_delegate_model(
            None,
            None,
            None,
            None,
            true,
            Some(BudgetBand::Ok),
            &all_configured,
            Some(("openai", "gpt-5.4-mini")),
        );
        assert_eq!(d.source, DelegateSource::Session);
        assert_eq!(d.role_model, None);
        assert!(d.refused.is_none());
        assert!(d.receipt.contains("no workflow role"), "{}", d.receipt);
    }

    /// The knob restores the old behaviour — deliberately, and on the record.
    #[test]
    fn escalation_runs_only_when_the_knob_is_on() {
        let call = |allow| {
            decide_delegate_model(
                Some(WorkflowRole::Orchestrate),
                None,
                None,
                Some(rm("anthropic", "claude-fable-5")),
                allow,
                Some(BudgetBand::Ok),
                &all_configured,
                Some(("openai", "gpt-5.4-mini")),
            )
        };
        assert_eq!(call(false).source, DelegateSource::Session);
        let on = call(true);
        assert_eq!(on.source, DelegateSource::Escalated);
        assert_eq!(on.role_model, Some(rm("anthropic", "claude-fable-5")));
        assert!(on.receipt.contains("escalation"), "{}", on.receipt);
    }

    /// (a) escalation stays inside the operator's configured providers.
    #[test]
    fn escalation_never_leaves_the_configured_providers() {
        let d = decide_delegate_model(
            Some(WorkflowRole::Orchestrate),
            None,
            None,
            Some(rm("anthropic", "claude-fable-5")),
            true,
            Some(BudgetBand::Ok),
            &only_openai,
            Some(("openai", "gpt-5.4-mini")),
        );
        assert_eq!(d.source, DelegateSource::Session);
        assert_eq!(
            d.refused.map(|(_, why)| why),
            Some(EscalationRefusal::UnconfiguredProvider)
        );
    }

    /// (b) spend caps, fail closed — including an unknown band.
    #[test]
    fn escalation_fails_closed_on_spend() {
        for band in [
            Some(BudgetBand::Soft),
            Some(BudgetBand::Gate),
            Some(BudgetBand::Hard),
            None,
        ] {
            let d = decide_delegate_model(
                Some(WorkflowRole::Orchestrate),
                None,
                None,
                Some(rm("anthropic", "claude-fable-5")),
                true,
                band,
                &all_configured,
                Some(("openai", "gpt-5.4-mini")),
            );
            assert_eq!(
                d.source,
                DelegateSource::Session,
                "band {band:?} must not escalate"
            );
            assert_eq!(
                d.refused.map(|(_, why)| why),
                Some(EscalationRefusal::SpendCapped),
                "band {band:?}"
            );
        }
    }

    /// (c) every outcome carries a receipt naming the model and why.
    #[test]
    fn every_decision_carries_a_receipt() {
        let cases = [
            decide_delegate_model(
                Some(WorkflowRole::Edit),
                Some(rm("openai", "gpt-5.4-mini")),
                None,
                None,
                false,
                Some(BudgetBand::Ok),
                &all_configured,
                Some(("openai", "gpt-5.4-mini")),
            ),
            decide_delegate_model(
                Some(WorkflowRole::Edit),
                None,
                Some(rm("openai", "gpt-5.4-mini")),
                None,
                false,
                Some(BudgetBand::Ok),
                &all_configured,
                Some(("openai", "gpt-5.4-mini")),
            ),
            decide_delegate_model(
                Some(WorkflowRole::Edit),
                None,
                None,
                Some(rm("anthropic", "claude-fable-5")),
                false,
                Some(BudgetBand::Ok),
                &all_configured,
                Some(("openai", "gpt-5.4-mini")),
            ),
        ];
        for d in cases {
            assert!(!d.receipt.is_empty());
            let json = d.receipt_json();
            assert_eq!(json["source"], d.source.as_str());
            assert!(json["summary"].as_str().is_some_and(|s| !s.is_empty()));
        }
    }

    /// The refusal receipt is JSON-shaped for the snapshot, naming what was declined.
    #[test]
    fn a_refusal_is_in_the_receipt_json() {
        let d = decide_delegate_model(
            Some(WorkflowRole::Edit),
            None,
            None,
            Some(rm("anthropic", "claude-fable-5")),
            false,
            Some(BudgetBand::Ok),
            &all_configured,
            Some(("openai", "gpt-5.4-mini")),
        );
        let json = d.receipt_json();
        assert_eq!(json["escalation_refused"]["model"], "claude-fable-5");
        assert_eq!(json["escalation_refused"]["reason"], "escalation_disabled");
        assert_eq!(json["role"], "edit");
    }

    /// Every workflow role reads a pack, so "I pinned all four packs" leaves no
    /// role free to route elsewhere. Review shares the frontier-judgment rung
    /// with Orchestrate; the ladder has no review rung of its own.
    #[test]
    fn every_role_pins_through_a_pack() {
        use super::super::packs::Role;
        assert_eq!(
            pack_role_for_workflow_role(WorkflowRole::Review),
            Some(Role::Hard)
        );
        assert_eq!(
            pack_role_for_workflow_role(WorkflowRole::Orchestrate),
            Some(Role::Hard)
        );
        assert_eq!(
            pack_role_for_workflow_role(WorkflowRole::Edit),
            Some(Role::Edit)
        );
        assert_eq!(
            pack_role_for_workflow_role(WorkflowRole::Mechanical),
            Some(Role::Mechanical)
        );
        assert_eq!(
            pack_role_for_workflow_role(WorkflowRole::Local),
            Some(Role::Local)
        );
        for role in WorkflowRole::all() {
            assert!(
                pack_role_for_workflow_role(role).is_some(),
                "{role:?} has no pack to pin through — a fully-pinned run would leak"
            );
        }
    }
}
